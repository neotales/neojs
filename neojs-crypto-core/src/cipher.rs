//! Bounded, stateful AES-256-CBC and AES-256-GCM ciphers.
//!
//! Provides [`Encryptor`], [`Decryptor`], [`CipherState`], [`CipherTable`],
//! and [`CipherBudget`] for bounded, buffered cipher execution.
//!
//! * **AES-256-CBC** (`aes-256-cbc`) uses a 32-byte key and 16-byte IV,
//!   supports strict PKCS#7 padding (default) or explicit no-padding
//!   ([`Encryptor::set_auto_padding`]), and is **not authenticated**.
//! * **AES-256-GCM** (`aes-256-gcm`) uses a 32-byte key and 12-byte nonce,
//!   supports associated data (`set_aad`) before any `update` or `finalize`,
//!   and emits/verifies a 16-byte authentication tag (`auth_tag` /
//!   `set_auth_tag`). Never reuse a 12-byte nonce under one key.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use aes_gcm::Aes256Gcm;
use aes_gcm::Nonce;
use aes_gcm::Tag;
use aes_gcm::aead::AeadInPlace;
use aes_gcm::aead::KeyInit as AeadKeyInit;
use cbc::cipher::BlockModeDecrypt;
use cbc::cipher::BlockModeEncrypt;
use cbc::cipher::KeyIvInit;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::block_padding::Pkcs7;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::envelope::KEY_LEN;
use crate::envelope::NONCE_LEN;
use crate::envelope::TAG_LEN;

/// CBC block and IV length in bytes.
pub const CBC_IV_LEN: usize = 16;
/// GCM nonce length in bytes.
pub const GCM_IV_LEN: usize = NONCE_LEN;
/// Largest cipher input accepted by one cipher object.
pub const MAX_CIPHER_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Largest GCM associated-data value accepted by one cipher object.
pub const MAX_CIPHER_AAD_BYTES: usize = 8 * 1024 * 1024;
/// Largest number of cipher objects that may be open at once.
pub const MAX_OPEN_CIPHERS: usize = 64;
/// Largest combined buffered input and associated data across open objects.
pub const MAX_OPEN_CIPHER_BYTES: usize = 64 * 1024 * 1024;

/// Supported stateful cipher algorithms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CipherAlgorithm {
    /// AES-256 in CBC mode (unauthenticated).
    Aes256Cbc,
    /// AES-256 in GCM mode (AEAD).
    Aes256Gcm,
}

impl CipherAlgorithm {
    /// Parse an algorithm name case-insensitively (`aes-256-cbc` or
    /// `aes-256-gcm`).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("aes-256-cbc") {
            Some(Self::Aes256Cbc)
        } else if name.eq_ignore_ascii_case("aes-256-gcm") {
            Some(Self::Aes256Gcm)
        } else {
            None
        }
    }

    /// Required IV or nonce length in bytes.
    #[must_use]
    pub const fn iv_len(self) -> usize {
        match self {
            Self::Aes256Cbc => CBC_IV_LEN,
            Self::Aes256Gcm => GCM_IV_LEN,
        }
    }
}

/// Cipher operation direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CipherDirection {
    /// Encrypt plaintext to ciphertext.
    Encrypt,
    /// Decrypt ciphertext to plaintext.
    Decrypt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Buffering,
    Finalized,
}

/// Per-object size bounds, overridable in tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CipherLimits {
    /// Largest buffered input accepted by one cipher object.
    pub max_input: usize,
    /// Largest GCM associated-data buffer accepted by one cipher object.
    pub max_aad: usize,
}

impl Default for CipherLimits {
    fn default() -> Self {
        Self {
            max_input: MAX_CIPHER_INPUT_BYTES,
            max_aad: MAX_CIPHER_AAD_BYTES,
        }
    }
}

/// Stable machine-readable failure category for [`CipherError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum CipherErrorCode {
    /// The algorithm name is not one of the two supported names.
    Algorithm,
    /// The key is not exactly 32 bytes.
    Key,
    /// The IV or nonce is not the algorithm's required length.
    Iv,
    /// The tag is not exactly 16 bytes.
    Tag,
    /// Decryption was finalized without setting a tag.
    MissingTag,
    /// The GCM tag did not verify.
    Authentication,
    /// CBC padding or a no-padding block length is invalid.
    Padding,
    /// The operation is not valid in the object's current state.
    State,
    /// The cipher handle was closed.
    Closed,
    /// The operation is not supported for this algorithm or direction.
    Unsupported,
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// Too many cipher objects are open.
    TooManyObjects,
    /// The aggregate cipher buffer budget is full.
    Busy,
    /// An internal invariant failed.
    Internal,
}

impl CipherErrorCode {
    /// Stable `NEO_CRYPTO_*` code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Algorithm => "NEO_CRYPTO_ALGORITHM",
            Self::Key => "NEO_CRYPTO_KEY",
            Self::Iv => "NEO_CRYPTO_IV",
            Self::Tag | Self::MissingTag => "NEO_CRYPTO_TAG",
            Self::Authentication => "NEO_CRYPTO_AUTH",
            Self::Padding => "NEO_CRYPTO_PADDING",
            Self::State => "NEO_CRYPTO_STATE",
            Self::Closed => "NEO_CRYPTO_CLOSED",
            Self::Unsupported => "NEO_CRYPTO_UNSUPPORTED",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::TooManyObjects => "NEO_CRYPTO_TOO_MANY_OBJECTS",
            Self::Busy => "NEO_CRYPTO_BUSY",
            Self::Internal => "NEO_CRYPTO_INTERNAL",
        }
    }

    /// Fixed human-readable message for this failure category.
    #[must_use]
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Algorithm => "unsupported cipher algorithm",
            Self::Key => "key must be exactly 32 bytes",
            Self::Iv => "iv has the wrong length for the cipher algorithm",
            Self::Tag => "authentication tag must be exactly 16 bytes",
            Self::MissingTag => "authentication tag is required before final",
            Self::Authentication => "authentication failed",
            Self::Padding => "invalid cipher padding or block length",
            Self::State => "cipher operation is not valid in the current state",
            Self::Closed => "the cipher object is closed",
            Self::Unsupported => "operation is not supported for this cipher",
            Self::Limit => "input exceeds the cipher size limit",
            Self::TooManyObjects => "too many cipher objects are open",
            Self::Busy => "too much cipher buffering is pending",
            Self::Internal => "the cipher operation failed internally",
        }
    }
}

impl fmt::Display for CipherErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure returned by stateful cipher operations.
///
/// Messages are fixed per [`CipherErrorCode`] and never echo keys, IVs,
/// plaintexts, ciphertexts, or tags.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{}", .code.default_message())]
#[non_exhaustive]
pub struct CipherError {
    code: CipherErrorCode,
}

impl CipherError {
    /// Create an error for `code`.
    #[must_use]
    pub const fn new(code: CipherErrorCode) -> Self {
        Self { code }
    }

    /// The stable failure category.
    #[must_use]
    pub const fn code(self) -> CipherErrorCode {
        self.code
    }

    /// Fixed message safe to expose to callers.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.code.default_message()
    }
}

#[derive(Default)]
struct CipherBudgetState {
    open: usize,
    bytes: usize,
}

/// Shared open-object and aggregate-buffer budget for stateful ciphers.
pub struct CipherBudget {
    max_open: usize,
    max_bytes: usize,
    state: Mutex<CipherBudgetState>,
}

impl Default for CipherBudget {
    fn default() -> Self {
        Self::with_limits(MAX_OPEN_CIPHERS, MAX_OPEN_CIPHER_BYTES)
    }
}

impl CipherBudget {
    /// Create a budget with explicit open-object and aggregate-byte caps.
    #[must_use]
    pub fn with_limits(max_open: usize, max_bytes: usize) -> Self {
        Self {
            max_open,
            max_bytes,
            state: Mutex::new(CipherBudgetState::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CipherBudgetState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reserve one open cipher slot, releasing it when the returned guard drops.
    pub fn reserve_open(self: &Arc<Self>) -> Result<OpenReservation, CipherError> {
        let mut state = self.lock();
        if state.open >= self.max_open {
            return Err(CipherError::new(CipherErrorCode::TooManyObjects));
        }
        state.open += 1;
        Ok(OpenReservation {
            budget: Arc::clone(self),
        })
    }

    fn charge(self: &Arc<Self>, bytes: usize) -> Result<(), CipherError> {
        if bytes == 0 {
            return Ok(());
        }
        let mut state = self.lock();
        let next = state
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| CipherError::new(CipherErrorCode::Busy))?;
        if next > self.max_bytes {
            return Err(CipherError::new(CipherErrorCode::Busy));
        }
        state.bytes = next;
        Ok(())
    }

    fn release(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let mut state = self.lock();
        state.bytes = state.bytes.saturating_sub(bytes);
    }
}

/// RAII open-object reservation released on drop.
pub struct OpenReservation {
    budget: Arc<CipherBudget>,
}

impl Drop for OpenReservation {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.open = state.open.saturating_sub(1);
    }
}

struct Buffered {
    budget: Arc<CipherBudget>,
    max: usize,
    bytes: Zeroizing<Vec<u8>>,
    charged: usize,
}

impl Buffered {
    fn new(budget: Arc<CipherBudget>, max: usize) -> Self {
        Self {
            budget,
            max,
            bytes: Zeroizing::new(Vec::new()),
            charged: 0,
        }
    }

    fn from_vec(
        budget: Arc<CipherBudget>,
        max: usize,
        bytes: Vec<u8>,
    ) -> Result<Self, CipherError> {
        let bytes = Zeroizing::new(bytes);
        if bytes.len() > max {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        budget.charge(bytes.len())?;
        let charged = bytes.len();
        Ok(Self {
            budget,
            max,
            bytes,
            charged,
        })
    }

    fn append(&mut self, data: &[u8]) -> Result<(), CipherError> {
        let next = self
            .bytes
            .len()
            .checked_add(data.len())
            .ok_or_else(|| CipherError::new(CipherErrorCode::Limit))?;
        if next > self.max {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        if data.is_empty() {
            return Ok(());
        }
        self.budget.charge(data.len())?;
        if self.bytes.try_reserve(data.len()).is_err() {
            self.budget.release(data.len());
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        self.bytes.extend_from_slice(data);
        self.charged += data.len();
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn into_bytes(mut self) -> Zeroizing<Vec<u8>> {
        let bytes = std::mem::take(&mut self.bytes);
        self.budget.release(self.charged);
        self.charged = 0;
        bytes
    }
}

impl Drop for Buffered {
    fn drop(&mut self) {
        if self.charged > 0 {
            self.budget.release(self.charged);
            self.charged = 0;
        }
    }
}

/// Core stateful cipher machine shared by [`Encryptor`], [`Decryptor`],
/// [`CipherTable`], and runtime resource wrappers.
pub struct CipherState {
    algorithm: CipherAlgorithm,
    direction: CipherDirection,
    key: Zeroizing<[u8; KEY_LEN]>,
    iv: Zeroizing<Vec<u8>>,
    padding: bool,
    stage: Stage,
    saw_update: bool,
    input: Option<Buffered>,
    aad: Option<Buffered>,
    tag: Option<[u8; TAG_LEN]>,
    limits: CipherLimits,
    budget: Arc<CipherBudget>,
}

impl CipherState {
    /// Construct a new cipher state with explicit `limits` and `budget`.
    pub fn new(
        direction: CipherDirection,
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
        limits: CipherLimits,
        budget: Arc<CipherBudget>,
    ) -> Result<Self, CipherError> {
        if key.len() != KEY_LEN {
            return Err(CipherError::new(CipherErrorCode::Key));
        }
        if iv.len() != algorithm.iv_len() {
            return Err(CipherError::new(CipherErrorCode::Iv));
        }
        let mut key_bytes = Zeroizing::new([0u8; KEY_LEN]);
        key_bytes.copy_from_slice(key);
        let iv_bytes = Zeroizing::new(iv.to_vec());
        Ok(Self {
            algorithm,
            direction,
            key: key_bytes,
            iv: iv_bytes,
            padding,
            stage: Stage::Buffering,
            saw_update: false,
            input: Some(Buffered::new(Arc::clone(&budget), limits.max_input)),
            aad: None,
            tag: None,
            limits,
            budget,
        })
    }

    /// Configured algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> CipherAlgorithm {
        self.algorithm
    }

    /// Configured per-object size limits.
    #[must_use]
    pub const fn limits(&self) -> CipherLimits {
        self.limits
    }

    /// Bytes this object may still accept through [`Self::update`].
    #[must_use]
    pub fn remaining_input(&self) -> usize {
        self.input
            .as_ref()
            .map(|input| self.limits.max_input.saturating_sub(input.bytes.len()))
            .unwrap_or(0)
    }

    /// Append input bytes into the bounded buffer.
    pub fn update(&mut self, data: &[u8]) -> Result<(), CipherError> {
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::State))?;
        input.append(data)?;
        self.saw_update = true;
        Ok(())
    }

    /// Set GCM associated data once, before any [`Self::update`] or
    /// [`Self::finalize`].
    pub fn set_aad(&mut self, aad: Vec<u8>) -> Result<(), CipherError> {
        if self.algorithm != CipherAlgorithm::Aes256Gcm {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Buffering || self.saw_update || self.aad.is_some() {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        if aad.len() > self.limits.max_aad {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        self.aad = Some(Buffered::from_vec(
            Arc::clone(&self.budget),
            self.limits.max_aad,
            aad,
        )?);
        Ok(())
    }

    /// Set the 16-byte authentication tag for GCM decryption before
    /// [`Self::finalize`].
    pub fn set_tag(&mut self, tag: &[u8]) -> Result<(), CipherError> {
        if self.algorithm != CipherAlgorithm::Aes256Gcm
            || self.direction != CipherDirection::Decrypt
        {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        if tag.len() != TAG_LEN {
            return Err(CipherError::new(CipherErrorCode::Tag));
        }
        if self.tag.is_some() {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        let mut bytes = [0u8; TAG_LEN];
        bytes.copy_from_slice(tag);
        self.tag = Some(bytes);
        Ok(())
    }

    /// Enable or disable PKCS#7 padding for CBC before [`Self::finalize`].
    pub fn set_padding(&mut self, padding: bool) -> Result<(), CipherError> {
        if self.algorithm != CipherAlgorithm::Aes256Cbc {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        self.padding = padding;
        Ok(())
    }

    /// Retrieve the 16-byte GCM authentication tag after [`Self::finalize`].
    pub fn auth_tag(&self) -> Result<[u8; TAG_LEN], CipherError> {
        if self.algorithm != CipherAlgorithm::Aes256Gcm
            || self.direction != CipherDirection::Encrypt
        {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Finalized {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        self.tag
            .ok_or_else(|| CipherError::new(CipherErrorCode::State))
    }

    /// Consume the buffered message, run the cipher, and return all output bytes.
    pub fn finalize(&mut self) -> Result<Vec<u8>, CipherError> {
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        self.stage = Stage::Finalized;
        let input = self
            .input
            .take()
            .ok_or_else(|| CipherError::new(CipherErrorCode::State))?;
        let aad = self.aad.take();
        if self.algorithm == CipherAlgorithm::Aes256Gcm
            && self.direction == CipherDirection::Decrypt
            && self.tag.is_none()
        {
            drop(input);
            return Err(CipherError::new(CipherErrorCode::MissingTag));
        }
        let aad_bytes = aad.as_ref().map(Buffered::as_slice).unwrap_or(&[]);
        match (self.algorithm, self.direction) {
            (CipherAlgorithm::Aes256Cbc, CipherDirection::Encrypt) => {
                cbc_encrypt(&*self.key, &self.iv, self.padding, &input.into_bytes())
            }
            (CipherAlgorithm::Aes256Cbc, CipherDirection::Decrypt) => {
                cbc_decrypt(&*self.key, &self.iv, self.padding, &input.into_bytes())
            }
            (CipherAlgorithm::Aes256Gcm, CipherDirection::Encrypt) => {
                let (output, tag) =
                    gcm_encrypt_zeroizing(&*self.key, &self.iv, aad_bytes, input.into_bytes())?;
                self.tag = Some(tag);
                Ok(output)
            }
            (CipherAlgorithm::Aes256Gcm, CipherDirection::Decrypt) => {
                let tag = self
                    .tag
                    .take()
                    .ok_or_else(|| CipherError::new(CipherErrorCode::MissingTag))?;
                gcm_decrypt_zeroizing(&*self.key, &self.iv, aad_bytes, input.into_bytes(), &tag)
            }
        }
    }
}

impl fmt::Debug for CipherState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CipherState")
            .field("algorithm", &self.algorithm)
            .field("direction", &self.direction)
            .field("stage", &self.stage)
            .field("key", &"[REDACTED]")
            .field("iv", &"[REDACTED]")
            .finish()
    }
}

/// Stateful AES-256-CBC / AES-256-GCM encryptor.
///
/// # Example
///
/// ```
/// use neojs_crypto_core::cipher::{CipherAlgorithm, Decryptor, Encryptor};
///
/// let key = [0x42u8; 32];
/// let iv = [0x11u8; 12];
/// let mut enc = Encryptor::new(CipherAlgorithm::Aes256Gcm, &key, &iv, false)?;
/// enc.set_aad(b"header".to_vec())?;
/// enc.update(b"hello ")?;
/// enc.update(b"world")?;
/// let ct = enc.finalize()?;
/// let tag = enc.auth_tag()?;
///
/// let mut dec = Decryptor::new(CipherAlgorithm::Aes256Gcm, &key, &iv, false)?;
/// dec.set_aad(b"header".to_vec())?;
/// dec.set_auth_tag(&tag)?;
/// dec.update(&ct)?;
/// assert_eq!(dec.finalize()?, b"hello world");
/// # Ok::<(), neojs_crypto_core::cipher::CipherError>(())
/// ```
pub struct Encryptor {
    state: CipherState,
    _open: Option<OpenReservation>,
}

impl Encryptor {
    /// Create a standalone encryptor with default per-object limits.
    pub fn new(
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
    ) -> Result<Self, CipherError> {
        let budget = Arc::new(CipherBudget::default());
        let state = CipherState::new(
            CipherDirection::Encrypt,
            algorithm,
            key,
            iv,
            padding,
            CipherLimits::default(),
            budget,
        )?;
        Ok(Self { state, _open: None })
    }

    /// Create an encryptor charged against a shared [`CipherBudget`].
    pub fn with_budget(
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
        limits: CipherLimits,
        budget: Arc<CipherBudget>,
    ) -> Result<Self, CipherError> {
        let open = budget.reserve_open()?;
        let state = CipherState::new(
            CipherDirection::Encrypt,
            algorithm,
            key,
            iv,
            padding,
            limits,
            budget,
        )?;
        Ok(Self {
            state,
            _open: Some(open),
        })
    }

    /// Configured cipher algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> CipherAlgorithm {
        self.state.algorithm()
    }

    /// Remaining input capacity in bytes.
    #[must_use]
    pub fn remaining_input(&self) -> usize {
        self.state.remaining_input()
    }

    /// Maximum associated data accepted in bytes.
    #[must_use]
    pub const fn max_aad(&self) -> usize {
        self.state.limits().max_aad
    }

    /// Buffer additional plaintext bytes.
    pub fn update(&mut self, data: &[u8]) -> Result<(), CipherError> {
        self.state.update(data)
    }

    /// Set GCM associated data once before any [`Self::update`] or [`Self::finalize`].
    pub fn set_aad(&mut self, aad: Vec<u8>) -> Result<(), CipherError> {
        self.state.set_aad(aad)
    }

    /// Enable or disable PKCS#7 padding for AES-256-CBC.
    pub fn set_auto_padding(&mut self, padding: bool) -> Result<(), CipherError> {
        self.state.set_padding(padding)
    }

    /// Finalize encryption and return the ciphertext.
    pub fn finalize(&mut self) -> Result<Vec<u8>, CipherError> {
        self.state.finalize()
    }

    /// Return the 16-byte GCM authentication tag after [`Self::finalize`].
    pub fn auth_tag(&self) -> Result<[u8; TAG_LEN], CipherError> {
        self.state.auth_tag()
    }
}

impl fmt::Debug for Encryptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Encryptor")
            .field("state", &self.state)
            .finish()
    }
}

/// Stateful AES-256-CBC / AES-256-GCM decryptor.
pub struct Decryptor {
    state: CipherState,
    _open: Option<OpenReservation>,
}

impl Decryptor {
    /// Create a standalone decryptor with default per-object limits.
    pub fn new(
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
    ) -> Result<Self, CipherError> {
        let budget = Arc::new(CipherBudget::default());
        let state = CipherState::new(
            CipherDirection::Decrypt,
            algorithm,
            key,
            iv,
            padding,
            CipherLimits::default(),
            budget,
        )?;
        Ok(Self { state, _open: None })
    }

    /// Create a decryptor charged against a shared [`CipherBudget`].
    pub fn with_budget(
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
        limits: CipherLimits,
        budget: Arc<CipherBudget>,
    ) -> Result<Self, CipherError> {
        let open = budget.reserve_open()?;
        let state = CipherState::new(
            CipherDirection::Decrypt,
            algorithm,
            key,
            iv,
            padding,
            limits,
            budget,
        )?;
        Ok(Self {
            state,
            _open: Some(open),
        })
    }

    /// Configured cipher algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> CipherAlgorithm {
        self.state.algorithm()
    }

    /// Remaining input capacity in bytes.
    #[must_use]
    pub fn remaining_input(&self) -> usize {
        self.state.remaining_input()
    }

    /// Maximum associated data accepted in bytes.
    #[must_use]
    pub const fn max_aad(&self) -> usize {
        self.state.limits().max_aad
    }

    /// Buffer additional ciphertext bytes.
    pub fn update(&mut self, data: &[u8]) -> Result<(), CipherError> {
        self.state.update(data)
    }

    /// Set GCM associated data once before any [`Self::update`] or [`Self::finalize`].
    pub fn set_aad(&mut self, aad: Vec<u8>) -> Result<(), CipherError> {
        self.state.set_aad(aad)
    }

    /// Set the 16-byte GCM authentication tag before [`Self::finalize`].
    pub fn set_auth_tag(&mut self, tag: &[u8]) -> Result<(), CipherError> {
        self.state.set_tag(tag)
    }

    /// Enable or disable PKCS#7 padding for AES-256-CBC.
    pub fn set_auto_padding(&mut self, padding: bool) -> Result<(), CipherError> {
        self.state.set_padding(padding)
    }

    /// Finalize decryption and return the verified plaintext.
    pub fn finalize(&mut self) -> Result<Vec<u8>, CipherError> {
        self.state.finalize()
    }
}

impl fmt::Debug for Decryptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Decryptor")
            .field("state", &self.state)
            .finish()
    }
}

struct CipherEntry {
    state: CipherState,
    _open: OpenReservation,
}

/// Engine-neutral bounded table of open [`CipherState`] handles.
///
/// Enforces [`MAX_OPEN_CIPHERS`] and [`MAX_OPEN_CIPHER_BYTES`] across all open
/// handles in the table without depending on a specific JS engine's resource
/// table.
pub struct CipherTable {
    budget: Arc<CipherBudget>,
    limits: CipherLimits,
    entries: HashMap<u32, CipherEntry>,
    next_id: u32,
}

impl Default for CipherTable {
    fn default() -> Self {
        Self::new(CipherLimits::default(), Arc::new(CipherBudget::default()))
    }
}

impl CipherTable {
    /// Create a table with explicit per-object `limits` and shared `budget`.
    #[must_use]
    pub fn new(limits: CipherLimits, budget: Arc<CipherBudget>) -> Self {
        Self {
            budget,
            limits,
            entries: HashMap::new(),
            next_id: 1,
        }
    }

    /// Number of currently open cipher handles.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no cipher handles are currently open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Create and insert a new cipher handle, returning its unique `u32` ID.
    pub fn create(
        &mut self,
        direction: CipherDirection,
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
    ) -> Result<u32, CipherError> {
        let open = self.budget.reserve_open()?;
        let state = CipherState::new(
            direction,
            algorithm,
            key,
            iv,
            padding,
            self.limits,
            Arc::clone(&self.budget),
        )?;
        let id = self.allocate_id()?;
        self.entries.insert(id, CipherEntry { state, _open: open });
        Ok(id)
    }

    fn allocate_id(&mut self) -> Result<u32, CipherError> {
        let start = self.next_id;
        loop {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 {
                self.next_id = 1;
            }
            if candidate != 0 && !self.entries.contains_key(&candidate) {
                return Ok(candidate);
            }
            if self.next_id == start {
                return Err(CipherError::new(CipherErrorCode::TooManyObjects));
            }
        }
    }

    /// Borrow the [`CipherState`] for `id`, or fail with [`CipherErrorCode::Closed`].
    pub fn get(&self, id: u32) -> Result<&CipherState, CipherError> {
        self.entries
            .get(&id)
            .map(|entry| &entry.state)
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))
    }

    /// Mutably borrow the [`CipherState`] for `id`, or fail with
    /// [`CipherErrorCode::Closed`].
    pub fn get_mut(&mut self, id: u32) -> Result<&mut CipherState, CipherError> {
        self.entries
            .get_mut(&id)
            .map(|entry| &mut entry.state)
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))
    }

    /// Close and release `id`. Idempotent: closing an already-closed ID is a no-op.
    pub fn close(&mut self, id: u32) {
        self.entries.remove(&id);
    }
}

/// CBC encryption with PKCS#7 padding or a strict no-padding mode.
pub fn cbc_encrypt(
    key: &[u8],
    iv: &[u8],
    padding: bool,
    plaintext: &[u8],
) -> Result<Vec<u8>, CipherError> {
    if key.len() != KEY_LEN {
        return Err(CipherError::new(CipherErrorCode::Key));
    }
    if iv.len() != CBC_IV_LEN {
        return Err(CipherError::new(CipherErrorCode::Iv));
    }
    let cipher = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv)
        .map_err(|_| CipherError::new(CipherErrorCode::Key))?;
    if !padding && !plaintext.len().is_multiple_of(CBC_IV_LEN) {
        return Err(CipherError::new(CipherErrorCode::Padding));
    }
    let output_len = if padding {
        ((plaintext.len() / CBC_IV_LEN) + 1)
            .checked_mul(CBC_IV_LEN)
            .ok_or_else(|| CipherError::new(CipherErrorCode::Limit))?
    } else {
        plaintext.len()
    };
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    output.resize(output_len, 0);
    let produced = if padding {
        cipher.encrypt_padded_b2b::<Pkcs7>(plaintext, &mut output)
    } else {
        cipher.encrypt_padded_b2b::<NoPadding>(plaintext, &mut output)
    }
    .map_err(|_| CipherError::new(CipherErrorCode::Padding))?;
    let length = produced.len();
    output.truncate(length);
    Ok(output)
}

/// CBC decryption with strict PKCS#7 removal or no padding.
pub fn cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    padding: bool,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CipherError> {
    if key.len() != KEY_LEN {
        return Err(CipherError::new(CipherErrorCode::Key));
    }
    if iv.len() != CBC_IV_LEN {
        return Err(CipherError::new(CipherErrorCode::Iv));
    }
    if !ciphertext.len().is_multiple_of(CBC_IV_LEN) {
        return Err(CipherError::new(CipherErrorCode::Padding));
    }
    if ciphertext.is_empty() {
        return if padding {
            Err(CipherError::new(CipherErrorCode::Padding))
        } else {
            Ok(Vec::new())
        };
    }
    let cipher = cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
        .map_err(|_| CipherError::new(CipherErrorCode::Key))?;
    let mut output = Zeroizing::new(Vec::new());
    output
        .try_reserve_exact(ciphertext.len())
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    output.resize(ciphertext.len(), 0);
    let produced = if padding {
        cipher.decrypt_padded_b2b::<Pkcs7>(ciphertext, &mut output)
    } else {
        cipher.decrypt_padded_b2b::<NoPadding>(ciphertext, &mut output)
    }
    .map_err(|_| CipherError::new(CipherErrorCode::Padding))?;
    let length = produced.len();
    output.truncate(length);
    Ok(std::mem::take(&mut *output))
}

fn gcm_cipher(key: &[u8]) -> Result<Aes256Gcm, CipherError> {
    if key.len() != KEY_LEN {
        return Err(CipherError::new(CipherErrorCode::Key));
    }
    Aes256Gcm::new_from_slice(key).map_err(|_| CipherError::new(CipherErrorCode::Key))
}

/// One-shot detached AES-256-GCM encryption.
pub fn gcm_encrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: Vec<u8>,
) -> Result<(Vec<u8>, [u8; TAG_LEN]), CipherError> {
    gcm_encrypt_zeroizing(key, nonce, aad, Zeroizing::new(plaintext))
}

fn gcm_encrypt_zeroizing(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    mut buffer: Zeroizing<Vec<u8>>,
) -> Result<(Vec<u8>, [u8; TAG_LEN]), CipherError> {
    if nonce.len() != GCM_IV_LEN {
        return Err(CipherError::new(CipherErrorCode::Iv));
    }
    let cipher = gcm_cipher(key)?;
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), aad, &mut buffer)
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    let mut tag_bytes = [0u8; TAG_LEN];
    tag_bytes.copy_from_slice(tag.as_slice());
    Ok((std::mem::take(&mut *buffer), tag_bytes))
}

/// One-shot detached AES-256-GCM decryption.
pub fn gcm_decrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext: Vec<u8>,
    tag: &[u8; TAG_LEN],
) -> Result<Vec<u8>, CipherError> {
    gcm_decrypt_zeroizing(key, nonce, aad, Zeroizing::new(ciphertext), tag)
}

fn gcm_decrypt_zeroizing(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    mut buffer: Zeroizing<Vec<u8>>,
    tag: &[u8; TAG_LEN],
) -> Result<Vec<u8>, CipherError> {
    if nonce.len() != GCM_IV_LEN {
        return Err(CipherError::new(CipherErrorCode::Iv));
    }
    let cipher = gcm_cipher(key)?;
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(nonce),
            aad,
            &mut buffer,
            Tag::from_slice(tag),
        )
        .map_err(|_| CipherError::new(CipherErrorCode::Authentication))?;
    Ok(std::mem::take(&mut *buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "hex length");
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    fn encode_hex(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            text.push_str(&format!("{byte:02x}"));
        }
        text
    }

    fn budget() -> Arc<CipherBudget> {
        Arc::new(CipherBudget::default())
    }

    fn state(
        direction: CipherDirection,
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
    ) -> CipherState {
        CipherState::new(
            direction,
            algorithm,
            key,
            iv,
            padding,
            CipherLimits::default(),
            budget(),
        )
        .expect("valid state")
    }

    fn state_with_budget(
        direction: CipherDirection,
        algorithm: CipherAlgorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
        limits: CipherLimits,
        budget: Arc<CipherBudget>,
    ) -> CipherState {
        CipherState::new(direction, algorithm, key, iv, padding, limits, budget)
            .expect("valid state")
    }

    fn tag_array(value: &[u8]) -> [u8; TAG_LEN] {
        value.try_into().expect("16-byte tag")
    }

    const NIST_CBC_KEY: &str = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4";
    const NIST_CBC_IV: &str = "000102030405060708090a0b0c0d0e0f";
    const NIST_CBC_PLAINTEXT: &str = concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710",
    );
    const NIST_CBC_CIPHERTEXT: &str = concat!(
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6",
        "9cfc4e967edb808d679f777bc6702c7d",
        "39f23369a9d9bacfa530e26304231461",
        "b2eb05e2c39be9fcda6c19078c6a9d1b",
    );

    const NIST_GCM_KEY: &str = "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308";
    const NIST_GCM_NONCE: &str = "cafebabefacedbaddecaf888";
    const NIST_GCM_AAD: &str = "feedfacedeadbeeffeedfacedeadbeefabaddad2";
    const NIST_GCM_PLAINTEXT: &str = concat!(
        "d9313225f88406e5a55909c5aff5269a",
        "86a7a9531534f7da2e4c303d8a318a72",
        "1c3c0c95956809532fcf0e2449a6b525",
        "b16aedf5aa0de657ba637b39",
    );
    const NIST_GCM_CIPHERTEXT: &str = concat!(
        "522dc1f099567d07f47f37a32a84427d",
        "643a8cdcbfe5c0c97598a2bd2555d1aa",
        "8cb08e48590dbb3da7b08b1056828838",
        "c5f61e6393ba7a0abcc9f662",
    );
    const NIST_GCM_TAG: &str = "76fc6ece0f4e1768cddf8853bb2d551b";

    const NODE_CBC_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const NODE_CBC_IV: &str = "101112131415161718191a1b1c1d1e1f";
    const NODE_CBC_PLAINTEXT: &str = "6e656f6a73206369706865722063626320696e7465726f70";
    const NODE_CBC_CIPHERTEXT: &str =
        "5226084e220371598b38cb20eaa1b85c2ff1b252517261b6f0054232236daa85";

    const NODE_GCM_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const NODE_GCM_IV: &str = "a1b2c3d4e5f60718293a4b5c";
    const NODE_GCM_AAD: &str = "6e656f6a732d6369706865722d636f6e74657874";
    const NODE_GCM_PLAINTEXT: &str = "6e656f6a73206369706865722067636d20696e7465726f70";
    const NODE_GCM_CIPHERTEXT: &str = "6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1";
    const NODE_GCM_TAG: &str = "9ea6507bfbc87455b1f24edc000cc662";

    // Unit test: published NIST SP 800-38A CBC test vector.
    #[test]
    fn published_nist_cbc_vector_matches_both_directions() {
        let key = decode_hex(NIST_CBC_KEY);
        let iv = decode_hex(NIST_CBC_IV);
        let plaintext = decode_hex(NIST_CBC_PLAINTEXT);
        let expected = decode_hex(NIST_CBC_CIPHERTEXT);

        let ciphertext = cbc_encrypt(&key, &iv, false, &plaintext).unwrap();
        assert_eq!(encode_hex(&ciphertext), NIST_CBC_CIPHERTEXT);
        let recovered = cbc_decrypt(&key, &iv, false, &expected).unwrap();
        assert_eq!(recovered, plaintext);
    }

    // Unit test: published NIST GCM test vector.
    #[test]
    fn published_nist_gcm_vector_matches_both_directions() {
        let key = decode_hex(NIST_GCM_KEY);
        let nonce = decode_hex(NIST_GCM_NONCE);
        let aad = decode_hex(NIST_GCM_AAD);
        let plaintext = decode_hex(NIST_GCM_PLAINTEXT);
        let expected = decode_hex(NIST_GCM_CIPHERTEXT);
        let expected_tag = tag_array(&decode_hex(NIST_GCM_TAG));

        let (ciphertext, tag) = gcm_encrypt(&key, &nonce, &aad, plaintext.clone()).unwrap();
        assert_eq!(encode_hex(&ciphertext), NIST_GCM_CIPHERTEXT);
        assert_eq!(encode_hex(&tag), NIST_GCM_TAG);
        assert_eq!(
            gcm_decrypt(&key, &nonce, &aad, expected, &expected_tag).unwrap(),
            plaintext
        );
    }

    // Unit test: Node/OpenSSL CBC interop fixture.
    #[test]
    fn node_cbc_fixture_matches_through_the_state() {
        let key = decode_hex(NODE_CBC_KEY);
        let iv = decode_hex(NODE_CBC_IV);
        let plaintext = decode_hex(NODE_CBC_PLAINTEXT);

        let mut encrypt = state(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Cbc,
            &key,
            &iv,
            true,
        );
        encrypt.update(&plaintext).unwrap();
        let ciphertext = encrypt.finalize().unwrap();
        assert_eq!(encode_hex(&ciphertext), NODE_CBC_CIPHERTEXT);

        let mut decrypt = state(
            CipherDirection::Decrypt,
            CipherAlgorithm::Aes256Cbc,
            &key,
            &iv,
            true,
        );
        decrypt.update(&ciphertext).unwrap();
        assert_eq!(decrypt.finalize().unwrap(), plaintext);
    }

    // Unit test: Node/OpenSSL GCM interop fixture.
    #[test]
    fn node_gcm_fixture_matches_through_the_state() {
        let key = decode_hex(NODE_GCM_KEY);
        let iv = decode_hex(NODE_GCM_IV);
        let aad = decode_hex(NODE_GCM_AAD);
        let plaintext = decode_hex(NODE_GCM_PLAINTEXT);
        let expected_tag = decode_hex(NODE_GCM_TAG);

        let mut encrypt = state(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
        );
        encrypt.set_aad(aad.clone()).unwrap();
        encrypt.update(&plaintext).unwrap();
        let ciphertext = encrypt.finalize().unwrap();
        assert_eq!(encode_hex(&ciphertext), NODE_GCM_CIPHERTEXT);
        assert_eq!(encode_hex(&encrypt.auth_tag().unwrap()), NODE_GCM_TAG);

        let mut decrypt = state(
            CipherDirection::Decrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
        );
        decrypt.set_aad(aad).unwrap();
        decrypt.set_tag(&expected_tag).unwrap();
        decrypt.update(&ciphertext).unwrap();
        assert_eq!(decrypt.finalize().unwrap(), plaintext);
    }

    // Unit test: codec padding validation and no-padding bounds.
    #[test]
    fn cbc_padding_is_strict_and_padding_can_be_disabled() {
        let key = [0x31u8; KEY_LEN];
        let iv = [0x32u8; CBC_IV_LEN];

        let ciphertext = cbc_encrypt(&key, &iv, true, &[]).unwrap();
        assert_eq!(ciphertext.len(), CBC_IV_LEN);
        assert!(
            cbc_decrypt(&key, &iv, true, &ciphertext)
                .unwrap()
                .is_empty()
        );
        assert!(cbc_encrypt(&key, &iv, false, &[]).unwrap().is_empty());
        assert!(cbc_decrypt(&key, &iv, false, &[]).unwrap().is_empty());

        assert_eq!(
            cbc_encrypt(&key, &iv, false, &[0u8; 15])
                .unwrap_err()
                .code(),
            CipherErrorCode::Padding
        );
        assert_eq!(
            cbc_decrypt(&key, &iv, false, &[0u8; 15])
                .unwrap_err()
                .code(),
            CipherErrorCode::Padding
        );
        assert_eq!(
            cbc_decrypt(&key, &iv, true, &[]).unwrap_err().code(),
            CipherErrorCode::Padding
        );

        let block = [0x41u8; CBC_IV_LEN];
        let ciphertext = cbc_encrypt(&key, &iv, false, &block).unwrap();
        assert_eq!(
            cbc_decrypt(&key, &iv, true, &ciphertext)
                .unwrap_err()
                .code(),
            CipherErrorCode::Padding
        );

        let ciphertext = cbc_encrypt(&key, &iv, true, &block).unwrap();
        let mut tampered = ciphertext;
        let last = tampered.len() - 1;
        tampered[last] ^= 0x80;
        assert_eq!(
            cbc_decrypt(&key, &iv, true, &tampered).unwrap_err().code(),
            CipherErrorCode::Padding
        );
    }

    // Unit test: codec tag/AAD/key/nonce mismatch detection.
    #[test]
    fn gcm_rejects_bad_tags_keys_nonces_and_aad() {
        let key = decode_hex(NODE_GCM_KEY);
        let iv = decode_hex(NODE_GCM_IV);
        let aad = decode_hex(NODE_GCM_AAD);
        let ciphertext = decode_hex(NODE_GCM_CIPHERTEXT);
        let tag = tag_array(&decode_hex(NODE_GCM_TAG));

        let mut bad_tag = tag;
        bad_tag[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&key, &iv, &aad, ciphertext.clone(), &bad_tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        assert_eq!(
            gcm_decrypt(&key, &iv, b"other", ciphertext.clone(), &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        let mut wrong_nonce = iv.clone();
        wrong_nonce[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&key, &wrong_nonce, &aad, ciphertext.clone(), &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        let mut wrong_key = key.clone();
        wrong_key[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&wrong_key, &iv, &aad, ciphertext.clone(), &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        let mut tampered = ciphertext;
        tampered[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&key, &iv, &aad, tampered, &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
    }

    // Unit test: state machine enforcement for GCM tags.
    #[test]
    fn gcm_requires_a_tag_before_final_and_never_returns_plaintext_from_update() {
        let key = [0x41u8; KEY_LEN];
        let iv = [0x42u8; GCM_IV_LEN];
        let mut decrypt = state(
            CipherDirection::Decrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
        );
        decrypt.update(&[0u8; 16]).unwrap();
        assert_eq!(
            decrypt.finalize().unwrap_err().code(),
            CipherErrorCode::MissingTag
        );

        let mut encrypt = state(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
        );
        encrypt.update(&[0u8; 16]).unwrap();
        assert_eq!(
            encrypt.auth_tag().unwrap_err().code(),
            CipherErrorCode::State
        );
        encrypt.finalize().unwrap();
        assert!(encrypt.auth_tag().is_ok());
    }

    // Unit test: state machine ordering guards.
    #[test]
    fn lifecycle_rejects_out_of_order_operations() {
        let key = [0x51u8; KEY_LEN];
        let iv = [0x52u8; GCM_IV_LEN];
        let mut encrypt = state(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
        );
        encrypt.update(b"payload").unwrap();
        encrypt.finalize().unwrap();
        assert_eq!(
            encrypt.update(b"later").unwrap_err().code(),
            CipherErrorCode::State
        );
        assert_eq!(
            encrypt.finalize().unwrap_err().code(),
            CipherErrorCode::State
        );
        assert_eq!(
            encrypt.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::State
        );

        let mut decrypt = state(
            CipherDirection::Decrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
        );
        decrypt.set_aad(Vec::new()).unwrap();
        assert_eq!(
            decrypt.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::State
        );
        decrypt.update(b"ciphertext").unwrap();
        assert_eq!(
            decrypt.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::State
        );
        let mut tag = [0u8; TAG_LEN];
        tag[0] = 1;
        decrypt.set_tag(&tag).unwrap();
        assert_eq!(
            decrypt.set_tag(&tag).unwrap_err().code(),
            CipherErrorCode::State
        );
    }

    // Unit test: algorithm/direction unsupported method checks.
    #[test]
    fn unsupported_operations_are_reported_for_the_algorithm() {
        let key = [0x61u8; KEY_LEN];
        let cbc_iv = [0x62u8; CBC_IV_LEN];
        let gcm_iv = [0x63u8; GCM_IV_LEN];

        let mut cbc = state(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Cbc,
            &key,
            &cbc_iv,
            true,
        );
        assert_eq!(
            cbc.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::Unsupported
        );

        let mut gcm = state(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &gcm_iv,
            false,
        );
        assert_eq!(
            gcm.set_padding(false).unwrap_err().code(),
            CipherErrorCode::Unsupported
        );

        let decrypt = state(
            CipherDirection::Decrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &gcm_iv,
            false,
        );
        assert_eq!(
            decrypt.auth_tag().unwrap_err().code(),
            CipherErrorCode::Unsupported
        );
    }

    // Unit test: per-object bounds check.
    #[test]
    fn per_object_limits_are_checked_before_buffering() {
        let limits = CipherLimits {
            max_input: 4,
            max_aad: 2,
        };
        let key = [0x71u8; KEY_LEN];
        let iv = [0x72u8; GCM_IV_LEN];
        let mut cipher = state_with_budget(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            budget(),
        );
        cipher.set_aad(vec![1, 2]).unwrap();
        cipher.update(&[1, 2, 3, 4]).unwrap();
        assert_eq!(
            cipher.update(&[5]).unwrap_err().code(),
            CipherErrorCode::Limit
        );
        assert_eq!(cipher.remaining_input(), 0);

        let mut other = state_with_budget(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            budget(),
        );
        assert_eq!(
            other.set_aad(vec![1, 2, 3]).unwrap_err().code(),
            CipherErrorCode::Limit
        );
    }

    // Unit test: aggregate budget and open-object cap checks.
    #[test]
    fn aggregate_and_open_budgets_are_enforced() {
        let shared = Arc::new(CipherBudget::with_limits(2, 4));
        let key = [0x81u8; KEY_LEN];
        let iv = [0x82u8; GCM_IV_LEN];
        let limits = CipherLimits {
            max_input: 64,
            max_aad: 64,
        };
        let mut first = state_with_budget(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            Arc::clone(&shared),
        );
        first.update(&[1, 2, 3, 4]).unwrap();
        let mut second = state_with_budget(
            CipherDirection::Encrypt,
            CipherAlgorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            Arc::clone(&shared),
        );
        assert_eq!(
            second.update(&[5]).unwrap_err().code(),
            CipherErrorCode::Busy
        );
        drop(first);
        second.update(&[5]).unwrap();

        let small = Arc::new(CipherBudget::with_limits(1, 16));
        let first = small.reserve_open().unwrap();
        assert_eq!(
            small.reserve_open().err().unwrap().code(),
            CipherErrorCode::TooManyObjects
        );
        drop(first);
        assert!(small.reserve_open().is_ok());
    }

    // Unit test: CipherTable handle lifecycle and open-cap enforcement.
    #[test]
    fn cipher_table_manages_handles_and_close_releases_budget() {
        let budget = Arc::new(CipherBudget::with_limits(1, 32));
        let mut table = CipherTable::new(CipherLimits::default(), budget);
        let key = [0x91u8; KEY_LEN];
        let iv = [0x92u8; GCM_IV_LEN];

        let id = table
            .create(
                CipherDirection::Encrypt,
                CipherAlgorithm::Aes256Gcm,
                &key,
                &iv,
                false,
            )
            .unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(
            table
                .create(
                    CipherDirection::Encrypt,
                    CipherAlgorithm::Aes256Gcm,
                    &key,
                    &iv,
                    false
                )
                .unwrap_err()
                .code(),
            CipherErrorCode::TooManyObjects
        );
        table.get_mut(id).unwrap().update(b"abc").unwrap();
        let ct = table.get_mut(id).unwrap().finalize().unwrap();
        let tag = table.get(id).unwrap().auth_tag().unwrap();
        table.close(id);
        table.close(id);
        assert!(table.is_empty());
        assert_eq!(table.get(id).err().unwrap().code(), CipherErrorCode::Closed);

        let dec_id = table
            .create(
                CipherDirection::Decrypt,
                CipherAlgorithm::Aes256Gcm,
                &key,
                &iv,
                false,
            )
            .unwrap();
        table.get_mut(dec_id).unwrap().set_tag(&tag).unwrap();
        table.get_mut(dec_id).unwrap().update(&ct).unwrap();
        assert_eq!(table.get_mut(dec_id).unwrap().finalize().unwrap(), b"abc");
    }

    // Unit test: boundary check for key and IV lengths.
    #[test]
    fn key_and_iv_lengths_are_enforced() {
        let key = [0u8; KEY_LEN];
        let iv = [0u8; GCM_IV_LEN];
        for length in [0usize, 1, 15, 31, 33] {
            assert_eq!(
                CipherState::new(
                    CipherDirection::Encrypt,
                    CipherAlgorithm::Aes256Gcm,
                    &vec![0u8; length],
                    &iv,
                    false,
                    CipherLimits::default(),
                    budget(),
                )
                .err()
                .unwrap()
                .code(),
                CipherErrorCode::Key
            );
        }
        for length in [0usize, 1, 11, 13, 16] {
            assert_eq!(
                CipherState::new(
                    CipherDirection::Encrypt,
                    CipherAlgorithm::Aes256Gcm,
                    &key,
                    &vec![0u8; length],
                    false,
                    CipherLimits::default(),
                    budget(),
                )
                .err()
                .unwrap()
                .code(),
                CipherErrorCode::Iv
            );
        }
        for length in [0usize, 12, 15, 17] {
            assert_eq!(
                CipherState::new(
                    CipherDirection::Encrypt,
                    CipherAlgorithm::Aes256Cbc,
                    &key,
                    &vec![0u8; length],
                    true,
                    CipherLimits::default(),
                    budget(),
                )
                .err()
                .unwrap()
                .code(),
                CipherErrorCode::Iv
            );
        }
    }

    // Unit test: exact error code strings and message invariants.
    #[test]
    fn errors_have_stable_codes_and_safe_messages() {
        for (code, expected) in [
            (CipherErrorCode::Algorithm, "NEO_CRYPTO_ALGORITHM"),
            (CipherErrorCode::Key, "NEO_CRYPTO_KEY"),
            (CipherErrorCode::Iv, "NEO_CRYPTO_IV"),
            (CipherErrorCode::Tag, "NEO_CRYPTO_TAG"),
            (CipherErrorCode::MissingTag, "NEO_CRYPTO_TAG"),
            (CipherErrorCode::Authentication, "NEO_CRYPTO_AUTH"),
            (CipherErrorCode::Padding, "NEO_CRYPTO_PADDING"),
            (CipherErrorCode::State, "NEO_CRYPTO_STATE"),
            (CipherErrorCode::Closed, "NEO_CRYPTO_CLOSED"),
            (CipherErrorCode::Unsupported, "NEO_CRYPTO_UNSUPPORTED"),
            (CipherErrorCode::Limit, "NEO_CRYPTO_LIMIT"),
            (
                CipherErrorCode::TooManyObjects,
                "NEO_CRYPTO_TOO_MANY_OBJECTS",
            ),
            (CipherErrorCode::Busy, "NEO_CRYPTO_BUSY"),
            (CipherErrorCode::Internal, "NEO_CRYPTO_INTERNAL"),
        ] {
            let err = CipherError::new(code);
            assert_eq!(err.code().as_str(), expected);
            let message = err.message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
            assert_eq!(err.to_string(), message);
        }
    }

    // Unit test: key length checked before empty/IV/padding checks and redacted Debug.
    #[test]
    fn cbc_validates_key_length_before_empty_and_iv_and_redacts_debug() {
        assert_eq!(
            cbc_decrypt(b"bad", &[0u8; CBC_IV_LEN], false, b"")
                .unwrap_err()
                .code(),
            CipherErrorCode::Key
        );
        assert_eq!(
            cbc_decrypt(b"bad", &[0u8; 3], false, b"")
                .unwrap_err()
                .code(),
            CipherErrorCode::Key
        );
        assert_eq!(
            cbc_encrypt(b"bad", &[0u8; 3], false, b"")
                .unwrap_err()
                .code(),
            CipherErrorCode::Key
        );

        let key = [0xabu8; KEY_LEN];
        let iv = [0xcdu8; GCM_IV_LEN];
        let enc = Encryptor::new(CipherAlgorithm::Aes256Gcm, &key, &iv, false).unwrap();
        let dec = Decryptor::new(CipherAlgorithm::Aes256Gcm, &key, &iv, false).unwrap();
        let enc_dbg = format!("{enc:?}");
        let dec_dbg = format!("{dec:?}");
        assert!(enc_dbg.contains("[REDACTED]"));
        assert!(dec_dbg.contains("[REDACTED]"));
        assert!(!enc_dbg.contains("171"));
        assert!(!dec_dbg.contains("171"));
    }
}
