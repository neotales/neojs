//! UTC five-field cron parsing and the host side of `neo:cron`.
//!
//! The JavaScript module object is installed by [`crate::builtins`] and is
//! exposed as `Neo.getBuiltInModule("cron")`, `import ... from "cron"`, and
//! `import ... from "neo:cron"`. This file registers the private host ops the
//! module calls: it parses an expression, searches for the next occurrence,
//! sleeps until an occurrence, and cancels a job.
//!
//! # Syntax
//!
//! An expression is five whitespace-separated fields:
//! `minute hour day-of-month month day-of-week`. Leading, trailing, and repeated
//! whitespace is accepted. Each field is `*`, a single value, a `start-end`
//! range, or a list joined by commas, and any of those except a bare value may
//! carry a `/step` suffix. A step on a bare value (`5/10`) starts at that value
//! and continues to the field maximum, matching the common Unix reading.
//!
//! | Field | Values | Names |
//! | --- | --- | --- |
//! | Minute | `0`-`59` | none |
//! | Hour | `0`-`23` | none |
//! | Day of month | `1`-`31` | none |
//! | Month | `1`-`12` | `JAN`-`DEC`, case-insensitive; the three-letter and full English names |
//! | Day of week | `0`-`7`, where `0` and `7` are Sunday | `SUN`-`SAT`, case-insensitive; the three-letter and full English names |
//!
//! A range must not wrap: `start` must be less than or equal to `end`. A step
//! must be at least one and no greater than the field maximum. Empty fields,
//! empty list entries, more than five fields, signed or non-decimal numbers, and
//! the extended Vixie/Quartz syntax (`L`, `W`, `#`, `?`) are rejected. The whole
//! expression is bounded by [`MAX_EXPRESSION_BYTES`].
//!
//! # Day-of-month and day-of-week
//!
//! The rule is the POSIX/Vixie one, decided per expression:
//!
//! - Both fields are `*`: every day matches.
//! - Only one field is `*`: the other field alone decides.
//! - Neither field is `*`: a day matches when **either** field matches.
//!
//! Only a literal `*` counts as unrestricted. `*/2` in one of those fields is a
//! restriction, so `0 0 */2 * MON` matches every second day of the month **or**
//! any Monday.
//!
//! # Time zone and search bound
//!
//! Every occurrence is UTC. There is no local-time interpretation, no DST
//! handling, and no time-zone option. An occurrence is a whole UTC minute, so
//! seconds and milliseconds are always zero.
//!
//! [`Schedule::next_after`] starts at the first minute strictly after its
//! argument and searches at most [`MAX_SEARCH_DAYS`] days. An expression with no
//! occurrence in that window, such as `0 0 31 2 *`, yields `None` rather than
//! searching forever. The window is deliberately wide enough for the rare
//! February 29 schedules that skip a non-leap century.
//!
//! A result is also always a value JavaScript's `Date` can represent. The next
//! occurrence after the last representable instant has none, so it yields
//! `None` and `neo:cron`'s `next` returns `null` rather than an invalid `Date`.
//! An `after` argument outside the range is treated as the nearest boundary.
//!
//! # Scheduling and liveness
//!
//! A job owns a pending host op that sleeps until its next occurrence. That op
//! is event-loop work, so an undisposed job keeps the isolate alive until it is
//! disposed or the [`crate::Runtime`] is dropped. Disposing cancels the pending
//! sleep; a callback that is already running is not interrupted. A synchronous
//! callback blocks the isolate and therefore delays every timer. The scheduler
//! detects the lateness when the sleep resolves and reports skipped occurrences
//! rather than running a backlog.
//!
//! # Wall-clock changes
//!
//! The wait never anchors to a single relative sleep. It re-reads the host wall
//! clock at intervals of at most [`MAX_WAIT_SLICE_MS`] and resolves only once
//! the clock has reached the due time. A backward clock adjustment therefore
//! extends the wait instead of dispatching the occurrence early, and a forward
//! adjustment is noticed within one slice instead of after the original delay.
//! The JavaScript scheduler re-reads the clock once more before dispatch, so a
//! jump backward in the gap between the wait and the callback also postpones
//! the tick rather than running it early. A forward jump can still make a tick
//! late; the normal missed-occurrence rule then discards the stale tick instead
//! of replaying a backlog. These checks bound the damage from clock changes;
//! they do not make scheduling monotonic or independent of the system clock.
//!
//! The host ops do not log. Failures are returned as fixed messages with the
//! stable codes [`CronErrorCode`] documents; callback failures are the
//! JavaScript module's responsibility.
//!
//! # Test clock
//!
//! The off-by-default `test-support` feature installs a deterministic clock
//! through [`CronTestClock`]. It changes only the host's notion of "now" and
//! the wait op's sleep; the parser and `next` keep using their explicit
//! arguments. A normal build contains no clock hook.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use tokio::sync::Notify;

#[cfg(feature = "test-support")]
use std::sync::Mutex;

/// Largest cron expression accepted, in bytes.
///
/// The bound is applied before splitting or allocating, so a long input cannot
/// make parsing work unbounded.
pub(crate) const MAX_EXPRESSION_BYTES: usize = 512;

/// Largest number of live jobs one runtime may hold.
///
/// A job that is not disposed keeps a host resource and a pending sleep. The cap
/// bounds both. Disposing releases the slot.
pub(crate) const MAX_JOBS: usize = 1024;

/// The occurrence search window, in days.
///
/// 100 years covers the gap between February 29 in 2096 and 2104, which is the
/// largest gap a five-field UTC expression can produce.
pub(crate) const MAX_SEARCH_DAYS: i64 = 366 * 100;

/// Milliseconds in one minute. Cron occurrences are whole minutes.
const MS_PER_MINUTE: i64 = 60_000;

/// Milliseconds in one UTC day.
const MS_PER_DAY: i64 = 86_400_000;

/// The largest absolute epoch millisecond value a JavaScript `Date` represents.
const MAX_EPOCH_MS_I64: i64 = 8_640_000_000_000_000;

/// [`MAX_EPOCH_MS_I64`] as the `f64` used at the JavaScript boundary.
const MAX_EPOCH_MS: f64 = MAX_EPOCH_MS_I64 as f64;

/// The longest a wait may sleep before re-reading the wall clock.
///
/// One minute matches cron's resolution: a forward clock jump is noticed at the
/// next minute boundary, and a long wait costs at most one wake-up per minute.
const MAX_WAIT_SLICE_MS: u64 = 60_000;

/// Stable machine-readable failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CronErrorCode {
    /// The expression is not five fields, or a field is malformed or out of
    /// range.
    Expression,
    /// The expression or the number of live jobs exceeds a documented bound.
    Limit,
    /// The expression is valid but has no representable occurrence in the
    /// search window.
    NoOccurrence,
    /// A millisecond argument is not a finite number in the `Date` range.
    Argument,
}

impl CronErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Expression => "NEO_CRON_EXPRESSION",
            Self::Limit => "NEO_CRON_LIMIT",
            Self::NoOccurrence => "NEO_CRON_NO_OCCURRENCE",
            Self::Argument => "NEO_CRON_ARGUMENT",
        }
    }
}

/// A failure safe to return to JavaScript.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CronError {
    code: CronErrorCode,
    message: &'static str,
}

impl CronError {
    fn new(code: CronErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    fn expression(message: &'static str) -> Self {
        Self::new(CronErrorCode::Expression, message)
    }

    fn limit() -> Self {
        Self::new(
            CronErrorCode::Limit,
            "the cron expression or the number of jobs exceeds the neo:cron limit",
        )
    }

    fn no_occurrence() -> Self {
        Self::new(
            CronErrorCode::NoOccurrence,
            "the cron expression has no occurrence within the 100-year search window or the Date range",
        )
    }

    fn argument() -> Self {
        Self::new(
            CronErrorCode::Argument,
            "the time must be a finite number of epoch milliseconds within the Date range",
        )
    }

    pub(crate) fn code(self) -> CronErrorCode {
        self.code
    }

    pub(crate) fn message(self) -> &'static str {
        self.message
    }
}

/// One of the five fields.
#[derive(Clone, Copy)]
enum Field {
    Minute,
    Hour,
    DayOfMonth,
    Month,
    DayOfWeek,
}

impl Field {
    fn min(self) -> u32 {
        match self {
            Self::Minute | Self::Hour => 0,
            Self::DayOfMonth | Self::Month => 1,
            Self::DayOfWeek => 0,
        }
    }

    /// The largest value a plain field value may take.
    fn max(self) -> u32 {
        match self {
            Self::Minute => 59,
            Self::Hour => 23,
            Self::DayOfMonth => 31,
            Self::Month => 12,
            Self::DayOfWeek => 7,
        }
    }
}

/// A parsed five-field UTC cron expression.
///
/// The fields are bit sets. `day_of_month` and `day_of_week` also record whether
/// their source field was a literal `*`, because the POSIX/Vixie day rule
/// depends on that, not on whether the set happens to contain every day.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Schedule {
    minutes: u64,
    hours: u32,
    days_of_month: u32,
    months: u16,
    days_of_week: u8,
    day_of_month_any: bool,
    day_of_week_any: bool,
}

impl Schedule {
    /// Parses an expression, or reports why it is not one this module accepts.
    pub(crate) fn parse(expression: &str) -> Result<Self, CronError> {
        if expression.len() > MAX_EXPRESSION_BYTES {
            return Err(CronError::limit());
        }
        let fields = expression.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(CronError::expression(
                "a cron expression must have exactly five fields",
            ));
        }

        let minutes = parse_field(fields[0], Field::Minute)?;
        let hours = parse_field(fields[1], Field::Hour)?;
        let days_of_month = parse_field(fields[2], Field::DayOfMonth)?;
        let months = parse_field(fields[3], Field::Month)?;
        let days_of_week = parse_field(fields[4], Field::DayOfWeek)?;

        Ok(Self {
            minutes,
            hours: hours as u32,
            days_of_month: days_of_month as u32,
            months: months as u16,
            // Numeric 7 is the Sunday alias; fold it onto bit 0. Bit 0 is
            // otherwise the only Sunday bit, so a range such as `5-7` becomes
            // Friday, Saturday, Sunday.
            days_of_week: (days_of_week as u8 & !(1 << 7))
                | if days_of_week & (1 << 7) != 0 { 1 } else { 0 },
            day_of_month_any: fields[2] == "*",
            day_of_week_any: fields[4] == "*",
        })
    }

    /// The first occurrence strictly after `after_ms`, or `None`.
    ///
    /// `after_ms` is an epoch millisecond value. The result is a whole UTC
    /// minute and is always greater than `after_ms`. The search is bounded by
    /// [`MAX_SEARCH_DAYS`]. Negative values are before the epoch and are
    /// supported.
    ///
    /// Every result is within the JavaScript `Date` range. When the next
    /// occurrence would be later than the last representable instant, the
    /// result is `None`; an `after_ms` outside the range is treated as the
    /// nearest boundary before the search.
    pub(crate) fn next_after(&self, after_ms: i64) -> Option<i64> {
        // Bound the query so arithmetic below cannot overflow and so a query
        // outside the Date range still has a well-defined starting point.
        let after_ms = after_ms.clamp(-MAX_EPOCH_MS_I64, MAX_EPOCH_MS_I64);
        // No representable minute is strictly after the last instant.
        if after_ms == MAX_EPOCH_MS_I64 {
            return None;
        }
        let start = after_ms
            .div_euclid(MS_PER_MINUTE)
            .checked_mul(MS_PER_MINUTE)?
            .checked_add(MS_PER_MINUTE)?;
        let end = after_ms
            .checked_add(MAX_SEARCH_DAYS.checked_mul(MS_PER_DAY)?)?
            .min(MAX_EPOCH_MS_I64);
        let mut day = start.div_euclid(MS_PER_DAY);
        let last_day = end.div_euclid(MS_PER_DAY);
        let mut from = (start.rem_euclid(MS_PER_DAY) / MS_PER_MINUTE) as u32;

        while day <= last_day {
            let (year, month, day_of_month) = civil_from_days(day);
            if self.matches_day(year, month, day_of_month)
                && let Some(minute_of_day) = self.next_minute_of_day(from)
            {
                let candidate = day
                    .checked_mul(MS_PER_DAY)?
                    .checked_add(i64::from(minute_of_day).checked_mul(MS_PER_MINUTE)?)?;
                if candidate >= start && candidate <= end {
                    return Some(candidate);
                }
            }
            day = day.checked_add(1)?;
            from = 0;
        }
        None
    }

    /// Whether the date, ignoring the time of day, matches the schedule.
    fn matches_day(&self, year: i64, month: u32, day_of_month: u32) -> bool {
        if self.months & (1 << month) == 0 {
            return false;
        }
        let month_matches = self.days_of_month & (1 << day_of_month) != 0;
        let weekday = weekday_from_days(days_from_civil(year, month, day_of_month));
        let weekday_matches = self.days_of_week & (1 << weekday) != 0;
        match (self.day_of_month_any, self.day_of_week_any) {
            (true, true) => true,
            (true, false) => weekday_matches,
            (false, true) => month_matches,
            (false, false) => month_matches || weekday_matches,
        }
    }

    /// The first selected minute of the day at or after `from`, or `None`.
    ///
    /// `from` is a minute of the day in `0..1440`.
    fn next_minute_of_day(&self, from: u32) -> Option<u32> {
        let start_hour = from / 60;
        for hour in start_hour..24 {
            if self.hours & (1 << hour) == 0 {
                continue;
            }
            let start_minute = if hour == start_hour { from % 60 } else { 0 };
            for minute in start_minute..60 {
                if self.minutes & (1 << minute) != 0 {
                    return Some(hour * 60 + minute);
                }
            }
        }
        None
    }
}

/// Parses one field into a bit set covering values `0..=63`.
fn parse_field(text: &str, field: Field) -> Result<u64, CronError> {
    if text.is_empty() {
        return Err(CronError::expression("a cron field cannot be empty"));
    }
    let mut bits = 0u64;
    let mut saw_part = false;
    for part in text.split(',') {
        if part.is_empty() {
            return Err(CronError::expression(
                "a cron field cannot contain an empty list entry",
            ));
        }
        bits |= parse_part(part, field)?;
        saw_part = true;
    }
    if !saw_part || bits == 0 {
        return Err(CronError::expression("a cron field selects no values"));
    }
    Ok(bits)
}

/// Parses `*`, `value`, `start-end`, or any of those with `/step`.
fn parse_part(text: &str, field: Field) -> Result<u64, CronError> {
    let (base, step_text) = match text.split_once('/') {
        Some((base, step)) => (base, Some(step)),
        None => (text, None),
    };
    if base.is_empty() {
        return Err(CronError::expression(
            "a cron step must have a value or range before it",
        ));
    }
    let step = match step_text {
        Some(step) => parse_number(step, 1, field.max())?,
        None => 1,
    };

    let (start, end) = if base == "*" {
        (field.min(), field.max())
    } else if let Some((start, end)) = base.split_once('-') {
        (parse_value(start, field)?, parse_value(end, field)?)
    } else {
        let start = parse_value(base, field)?;
        // `5/10` means "from 5 to the field maximum every 10", not just 5.
        (
            start,
            if step_text.is_some() {
                field.max()
            } else {
                start
            },
        )
    };
    if start > end {
        return Err(CronError::expression(
            "a cron range must not wrap; start must not be greater than end",
        ));
    }

    let mut bits = 0u64;
    let mut value = start;
    while value <= end {
        bits |= 1u64 << value;
        match value.checked_add(step) {
            Some(next) => value = next,
            None => break,
        }
    }
    Ok(bits)
}

/// Parses a field value, accepting month and weekday names.
fn parse_value(text: &str, field: Field) -> Result<u32, CronError> {
    if text.is_empty() {
        return Err(CronError::expression("a cron value cannot be empty"));
    }
    match field {
        Field::Month => {
            if let Some(month) = month_name(text) {
                return Ok(month);
            }
        }
        Field::DayOfWeek => {
            if let Some(weekday) = weekday_name(text) {
                return Ok(weekday);
            }
        }
        _ => {}
    }
    parse_number(text, field.min(), field.max())
        .map_err(|_| CronError::expression("a cron field value is out of range or malformed"))
}

/// Parses an unsigned decimal with no sign and no more than two digits.
fn parse_number(text: &str, min: u32, max: u32) -> Result<u32, CronError> {
    if text.is_empty() || text.len() > 2 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CronError::expression(
            "a cron number must be one or two decimal digits",
        ));
    }
    let value = text
        .parse::<u32>()
        .map_err(|_| CronError::expression("a cron number is too large"))?;
    if value < min || value > max {
        return Err(CronError::expression("a cron field value is out of range"));
    }
    Ok(value)
}

/// `JAN`-`DEC`, case-insensitive, with the full English name accepted too.
fn month_name(text: &str) -> Option<u32> {
    const NAMES: [(&str, &str, u32); 12] = [
        ("JAN", "JANUARY", 1),
        ("FEB", "FEBRUARY", 2),
        ("MAR", "MARCH", 3),
        ("APR", "APRIL", 4),
        ("MAY", "MAY", 5),
        ("JUN", "JUNE", 6),
        ("JUL", "JULY", 7),
        ("AUG", "AUGUST", 8),
        ("SEP", "SEPTEMBER", 9),
        ("OCT", "OCTOBER", 10),
        ("NOV", "NOVEMBER", 11),
        ("DEC", "DECEMBER", 12),
    ];
    NAMES.iter().find_map(|(short, long, month)| {
        (text.eq_ignore_ascii_case(short) || text.eq_ignore_ascii_case(long)).then_some(*month)
    })
}

/// `SUN`-`SAT`, case-insensitive, with the full English name accepted too.
fn weekday_name(text: &str) -> Option<u32> {
    const NAMES: [(&str, &str, u32); 7] = [
        ("SUN", "SUNDAY", 0),
        ("MON", "MONDAY", 1),
        ("TUE", "TUESDAY", 2),
        ("WED", "WEDNESDAY", 3),
        ("THU", "THURSDAY", 4),
        ("FRI", "FRIDAY", 5),
        ("SAT", "SATURDAY", 6),
    ];
    NAMES.iter().find_map(|(short, long, weekday)| {
        (text.eq_ignore_ascii_case(short) || text.eq_ignore_ascii_case(long)).then_some(*weekday)
    })
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// Howard Hinnant's `days_from_civil` construction, written here rather than
/// pulled from a date crate so the module has no time-zone or calendar
/// dependency. The input is a real date produced by [`civil_from_days`] or by
/// the tests; invalid dates are not normalized.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The proleptic Gregorian date for days since 1970-01-01.
///
/// The inverse of [`days_from_civil`], used to turn a day index into a date
/// without `chrono` or the process time zone.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

/// Day of the week for days since 1970-01-01, where Sunday is 0.
///
/// 1970-01-01 was a Thursday, so the offset is 4.
fn weekday_from_days(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

/// Converts a JavaScript epoch millisecond value to an integer.
fn epoch_ms(value: f64) -> Result<i64, CronError> {
    if !value.is_finite() || value.abs() > MAX_EPOCH_MS {
        return Err(CronError::argument());
    }
    Ok(value.floor() as i64)
}

/// The current epoch millisecond time, or the installed test clock.
fn now_ms() -> f64 {
    #[cfg(feature = "test-support")]
    if let Some(clock) = test_clock() {
        return clock.now();
    }
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_millis() as f64,
        // A clock before the Unix epoch is not useful for scheduling, and a
        // negative value would make the first occurrence search strange.
        Err(_) => 0.0,
    }
}

/// A job's cancellation flag and wake-up notification.
struct CancelState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelState {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// A parsed job held in the resource table.
struct CronJob {
    schedule: Schedule,
    cancel: Arc<CancelState>,
}

/// The resource-table entry that owns a job's lifetime.
struct CronJobResource {
    job: Arc<CronJob>,
}

impl Resource for CronJobResource {
    fn name(&self) -> Cow<'_, str> {
        "cron".into()
    }

    fn close(self: Rc<Self>) {
        self.job.cancel.cancel();
    }
}

/// Per-runtime job count.
struct CronState {
    jobs: usize,
}

/// Milliseconds to sleep before the next wall-clock check, or `None` when due.
///
/// The slice is capped at [`MAX_WAIT_SLICE_MS`] so a forward clock jump is
/// noticed within one slice; a fraction of a millisecond still waits one whole
/// millisecond rather than spinning.
fn wait_slice_ms(now_ms: f64, due_ms: i64) -> Option<u64> {
    if now_ms >= due_ms as f64 {
        return None;
    }
    let remaining = (due_ms as f64 - now_ms).min(MAX_WAIT_SLICE_MS as f64);
    Some((remaining.ceil().max(1.0)) as u64)
}

/// Sleeps in bounded slices until the wall clock reaches `due_ms`.
///
/// `now` and `sleep` are injected so tests can move a deterministic clock
/// forward or backward without a real sleep. Returns `false` when the job is
/// cancelled, including a cancellation that arrives between slices.
async fn wait_until_with<Now, Sleep, SleepFuture>(
    due_ms: i64,
    cancel: &CancelState,
    mut now: Now,
    mut sleep: Sleep,
) -> bool
where
    Now: FnMut() -> f64,
    Sleep: FnMut(u64) -> SleepFuture,
    SleepFuture: std::future::Future<Output = ()>,
{
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let Some(slice_ms) = wait_slice_ms(now(), due_ms) else {
            return true;
        };
        let sleeping = sleep(slice_ms);
        tokio::select! {
            _ = sleeping => {}
            _ = cancel.wait() => return false,
        }
    }
}

/// Sleeps until `due_ms`, or returns `false` when the job is cancelled.
async fn wait_until(due_ms: i64, cancel: &CancelState) -> bool {
    #[cfg(feature = "test-support")]
    if let Some(clock) = test_clock() {
        return tokio::select! {
            _ = clock.wait_until(due_ms as f64) => true,
            _ = cancel.wait() => false,
        };
    }

    wait_until_with(due_ms, cancel, now_ms, |milliseconds| {
        tokio::time::sleep(std::time::Duration::from_millis(milliseconds))
    })
    .await
}

/// A response object with a null prototype, like the other built-in modules.
fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

fn set_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    set(scope, object, name, value.into());
}

fn set_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: f64,
) {
    set(scope, object, name, v8::Number::new(scope, value).into());
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: CronError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    set_string(scope, object, "code", error.code().as_str());
    set_string(scope, object, "message", error.message());
}

/// Create a job. Returns `{ ok: true, token, next }` or a failure object.
///
/// The expression is parsed once and stored, so later waits do not reparse it.
/// `next` is the first occurrence strictly after the host clock, which proves
/// the expression has a future occurrence before a job is admitted.
#[op2]
fn op_neo_cron_create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: Rc<RefCell<OpState>>,
    #[string] expression: String,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    let result = (|| {
        let schedule = Schedule::parse(&expression)?;
        let now = epoch_ms(now_ms())?;
        let next = schedule
            .next_after(now)
            .ok_or_else(CronError::no_occurrence)?;
        let mut state = state.borrow_mut();
        {
            let cron = state.borrow_mut::<CronState>();
            if cron.jobs >= MAX_JOBS {
                return Err(CronError::limit());
            }
            cron.jobs += 1;
        }
        let job = Arc::new(CronJob {
            schedule,
            cancel: Arc::new(CancelState::new()),
        });
        let token = state.resource_table.add(CronJobResource { job });
        Ok((token, next))
    })();
    match result {
        Ok((token, next)) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set_number(scope, object, "token", f64::from(token));
            set_number(scope, object, "next", next as f64);
            object.into()
        }
        Err(error) => {
            set_failure(scope, object, error);
            object.into()
        }
    }
}

/// Dispose a job by token. Returns `true` only for the first call.
#[op2(fast)]
fn op_neo_cron_dispose(state: Rc<RefCell<OpState>>, #[smi] token: ResourceId) -> bool {
    let mut state = state.borrow_mut();
    let Ok(resource) = state.resource_table.take::<CronJobResource>(token) else {
        return false;
    };
    resource.job.cancel.cancel();
    let cron = state.borrow_mut::<CronState>();
    cron.jobs = cron.jobs.saturating_sub(1);
    true
}

/// The next occurrence after an explicit time, for `neo:cron`'s `next`.
///
/// Returns `{ ok: true, value: number | null }` or a failure object. `null`
/// means the expression is valid but has no occurrence in the search window or
/// the representable `Date` range.
#[op2]
fn op_neo_cron_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    #[string] expression: String,
    after_ms: f64,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    let result = Schedule::parse(&expression).and_then(|schedule| {
        let after = epoch_ms(after_ms)?;
        Ok(schedule.next_after(after))
    });
    match result {
        Ok(Some(next)) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set_number(scope, object, "value", next as f64);
            object.into()
        }
        Ok(None) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(scope, object, "value", v8::null(scope).into());
            object.into()
        }
        Err(error) => {
            set_failure(scope, object, error);
            object.into()
        }
    }
}

/// The next occurrence after `after_ms` for a live job, or `NaN`.
#[op2(fast)]
fn op_neo_cron_following(
    state: Rc<RefCell<OpState>>,
    #[smi] token: ResourceId,
    after_ms: f64,
) -> f64 {
    let state = state.borrow();
    let Ok(resource) = state.resource_table.get::<CronJobResource>(token) else {
        return f64::NAN;
    };
    let Ok(after) = epoch_ms(after_ms) else {
        return f64::NAN;
    };
    match resource.job.schedule.next_after(after) {
        Some(next) => next as f64,
        None => f64::NAN,
    }
}

/// Sleep until `due_ms`. Returns the due time, or `-1` when cancelled.
///
/// A bad token or an out-of-range due time also returns `-1`, because the
/// JavaScript scheduler treats every non-positive result as "stop".
#[op2]
async fn op_neo_cron_wait(
    state: Rc<RefCell<OpState>>,
    #[smi] token: ResourceId,
    due_ms: f64,
) -> f64 {
    let job = {
        let state = state.borrow();
        match state.resource_table.get::<CronJobResource>(token) {
            Ok(resource) => resource.job.clone(),
            Err(_) => return -1.0,
        }
    };
    let Ok(due) = epoch_ms(due_ms) else {
        return -1.0;
    };
    if job.cancel.is_cancelled() {
        return -1.0;
    }
    if wait_until(due, &job.cancel).await {
        due as f64
    } else {
        -1.0
    }
}

/// The host clock in epoch milliseconds.
#[op2(fast)]
fn op_neo_cron_now() -> f64 {
    now_ms()
}

deno_core::extension!(
    neojs_cron,
    ops = [
        op_neo_cron_create,
        op_neo_cron_dispose,
        op_neo_cron_next,
        op_neo_cron_following,
        op_neo_cron_wait,
        op_neo_cron_now
    ],
    state = |state| {
        state.put(CronState { jobs: 0 });
    },
    docs = "UTC five-field cron parsing and scheduling behind `Neo.getBuiltInModule(\"cron\")`."
);

/// Advance the installed test clock. Compiled only with `test-support`.
///
/// Returns `false` when no clock is installed or the advance is not a finite,
/// non-negative number. The JavaScript tests use it to drive the scheduler
/// without waiting on wall-clock minutes.
#[cfg(feature = "test-support")]
#[op2(fast)]
fn op_neo_cron_test_advance(milliseconds: f64) -> bool {
    if !milliseconds.is_finite() || milliseconds < 0.0 {
        return false;
    }
    match test_clock() {
        Some(clock) => {
            clock.advance(milliseconds);
            true
        }
        None => false,
    }
}

#[cfg(feature = "test-support")]
deno_core::extension!(
    neojs_cron_test,
    ops = [op_neo_cron_test_advance],
    docs = "Test-only clock advance for `neo:cron`, compiled only with `test-support`."
);

/// The test-only `neo:cron` extension.
#[cfg(feature = "test-support")]
pub(crate) fn test_support_extension() -> deno_core::Extension {
    neojs_cron_test::init()
}

#[cfg(feature = "test-support")]
struct TestClock {
    now: Mutex<f64>,
    notify: Notify,
}

#[cfg(feature = "test-support")]
impl TestClock {
    fn now(&self) -> f64 {
        *self
            .now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn advance(&self, milliseconds: f64) {
        assert!(
            milliseconds.is_finite() && milliseconds >= 0.0,
            "a test clock advance must be a finite, non-negative number of milliseconds"
        );
        {
            let mut now = self
                .now
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *now += milliseconds;
        }
        self.notify.notify_waiters();
    }

    async fn wait_until(&self, until: f64) {
        loop {
            // Register before checking: on one thread no advance can happen
            // between these two statements, so a wake-up cannot be missed.
            let notified = self.notify.notified();
            if self.now() >= until {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(feature = "test-support")]
thread_local! {
    static TEST_CLOCK: RefCell<Option<Arc<TestClock>>> = const { RefCell::new(None) };
}

#[cfg(feature = "test-support")]
fn test_clock() -> Option<Arc<TestClock>> {
    TEST_CLOCK.with(|clock| clock.borrow().clone())
}

/// A deterministic clock for `neo:cron` tests, compiled only with
/// `test-support`.
///
/// Installs a frozen clock on the current thread. The runtime and its host ops
/// must be used on that thread, as [`crate::Runtime`] already requires. The
/// clock starts at `start_ms` epoch milliseconds and moves only when
/// [`CronTestClock::advance`] is called. Dropping the clock removes it if it is
/// still the installed one.
///
/// This is a test hook, not a public scheduling option. A normal build has no
/// clock hook and always sleeps against the operating system clock.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub struct CronTestClock {
    inner: Arc<TestClock>,
}

#[cfg(feature = "test-support")]
impl CronTestClock {
    /// Install a clock at `start_ms` on the current thread.
    pub fn install(start_ms: f64) -> Self {
        assert!(
            start_ms.is_finite(),
            "a test clock start must be a finite epoch millisecond value"
        );
        let clock = Self {
            inner: Arc::new(TestClock {
                now: Mutex::new(start_ms),
                notify: Notify::new(),
            }),
        };
        TEST_CLOCK.with(|slot| *slot.borrow_mut() = Some(clock.inner.clone()));
        clock
    }

    /// Move the clock forward by `milliseconds` and wake pending sleeps.
    pub fn advance(&self, milliseconds: f64) {
        self.inner.advance(milliseconds);
    }

    /// The current simulated epoch millisecond value.
    pub fn now_ms(&self) -> f64 {
        self.inner.now()
    }
}

#[cfg(feature = "test-support")]
impl Drop for CronTestClock {
    fn drop(&mut self) {
        TEST_CLOCK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|installed| Arc::ptr_eq(installed, &self.inner))
            {
                *slot = None;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::CancelState;
    use super::MAX_WAIT_SLICE_MS;
    use super::Schedule;
    use super::civil_from_days;
    use super::days_from_civil;
    use super::wait_slice_ms;
    use super::wait_until_with;
    use super::weekday_from_days;

    /// Builds an epoch millisecond value for a UTC date and time.
    fn at(year: i64, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        days_from_civil(year, month, day) * 86_400_000
            + i64::from(hour) * 3_600_000
            + i64::from(minute) * 60_000
    }

    fn next(expression: &str, after: i64) -> Option<i64> {
        Schedule::parse(expression)
            .expect("valid expression")
            .next_after(after)
    }

    /// Unit case (exact parser assertion): the calendar helpers are private
    /// arithmetic with real branching, and an error here shifts every
    /// occurrence.
    #[test]
    fn civil_dates_round_trip_and_use_sunday_zero() {
        for (year, month, day, weekday) in [
            (1970, 1, 1, 4),
            (2000, 2, 29, 2),
            (2024, 1, 1, 1),
            (2024, 12, 31, 2),
            (1900, 3, 1, 4),
        ] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
            assert_eq!(weekday_from_days(days), weekday);
        }
    }

    /// Unit case (exact parser assertion): accepted syntax and the value each
    /// field selects.
    #[test]
    fn parses_the_supported_syntax() {
        for expression in [
            "* * * * *",
            "0 0 * * *",
            "*/5 * * * *",
            "0-30/5 1,2,3 1/3 JAN-MAR MON-FRI",
            "59 23 31 12 7",
            "0 0 1 January Monday",
            "  0\t*  *   * *  ",
            "*,30 * * * *",
        ] {
            let schedule = Schedule::parse(expression).unwrap_or_else(|_| panic!("{expression}"));
            assert!(schedule.next_after(0).is_some(), "{expression}");
        }
    }

    /// Unit case (exact parser assertion): rejected shapes, including the
    /// extended syntax this module deliberately does not implement.
    #[test]
    fn rejects_malformed_and_extended_expressions() {
        for expression in [
            "",
            "* * * *",
            "* * * * * *",
            "*/0 * * * *",
            "*/60 * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * 32 * *",
            "* * * 0 *",
            "* * * 13 *",
            "* * * * 8",
            "5-1 * * * *",
            "1-2-3 * * * *",
            "1/2/3 * * * *",
            "1,,2 * * * *",
            "0 0 L * *",
            "0 0 15W * *",
            "0 0 * * MON#2",
            "0 0 ? * *",
            "+1 * * * *",
        ] {
            assert!(
                Schedule::parse(expression).is_err(),
                "{expression} should be rejected"
            );
        }
    }

    /// Unit case (exact parser assertion): a step on a bare value runs to the
    /// field maximum, and a plain value selects only itself.
    #[test]
    fn bare_value_steps_run_to_the_field_maximum() {
        let every_ten_from_five = Schedule::parse("5/10 * * * *").unwrap();
        let values = (0..60)
            .filter(|minute| every_ten_from_five.next_minute_of_day(*minute) == Some(*minute))
            .collect::<Vec<_>>();
        assert_eq!(values, vec![5, 15, 25, 35, 45, 55]);
    }

    /// Unit case (exact parser assertion): known next occurrences, including a
    /// leap day and the strict "after" boundary.
    #[test]
    fn computes_known_next_occurrences() {
        assert_eq!(
            next("* * * * *", at(2024, 1, 1, 0, 0)),
            Some(at(2024, 1, 1, 0, 1))
        );
        assert_eq!(
            next("*/15 * * * *", at(2024, 1, 1, 0, 0)),
            Some(at(2024, 1, 1, 0, 15))
        );
        assert_eq!(
            next("0 0 29 2 *", at(2023, 3, 1, 0, 0)),
            Some(at(2024, 2, 29, 0, 0))
        );
        assert_eq!(
            next("0 0 29 2 *", at(2025, 3, 1, 0, 0)),
            Some(at(2028, 2, 29, 0, 0))
        );
        assert_eq!(
            next("30 9 * * MON-FRI", at(2024, 1, 5, 9, 30)),
            Some(at(2024, 1, 8, 9, 30))
        );
        assert_eq!(
            next("0 0 * * 7", at(2024, 1, 5, 0, 0)),
            Some(at(2024, 1, 7, 0, 0))
        );
    }

    /// Unit case (exact parser assertion): the POSIX/Vixie day rule, including
    /// `*/n` counting as a restriction.
    #[test]
    fn applies_the_posix_day_of_month_and_weekday_rule() {
        // Both restricted: the 13th OR any Tuesday.
        assert_eq!(
            next("0 0 13 * TUE", at(2024, 5, 13, 1, 0)),
            Some(at(2024, 5, 14, 0, 0))
        );
        // Both restricted, day-of-month matches first.
        assert_eq!(
            next("0 0 13 * TUE", at(2024, 5, 12, 1, 0)),
            Some(at(2024, 5, 13, 0, 0))
        );
        // Only day-of-week restricted.
        assert_eq!(
            next("0 0 * * MON", at(2024, 1, 1, 0, 0)),
            Some(at(2024, 1, 8, 0, 0))
        );
        // `*/2` is a restriction and starts at day 1, so this is "odd days or
        // Monday". January 1 is both, but it is the excluded starting point;
        // the next match is January 3.
        assert_eq!(
            next("0 0 */2 * MON", at(2024, 1, 1, 0, 0)),
            Some(at(2024, 1, 3, 0, 0))
        );
    }

    /// Unit case (exact parser assertion): an impossible expression is bounded
    /// and returns `None` instead of searching forever.
    #[test]
    fn reports_no_occurrence_within_the_bound() {
        assert_eq!(next("0 0 31 2 *", at(2024, 1, 1, 0, 0)), None);
        assert_eq!(next("0 0 31 4 *", at(2024, 1, 1, 0, 0)), None);
    }

    /// Unit case (exact parser assertion): the expression length bound is
    /// enforced before parsing.
    #[test]
    fn bounds_the_expression_length() {
        let long = "1".repeat(super::MAX_EXPRESSION_BYTES + 1);
        assert!(Schedule::parse(&long).is_err());
    }

    /// Unit case (exact parser assertion): strict "after" means an occurrence
    /// itself is never returned twice.
    #[test]
    fn next_is_strictly_after_the_argument() {
        let occurrence = at(2024, 1, 1, 0, 0);
        assert_eq!(next("0 0 * * *", occurrence), Some(at(2024, 1, 2, 0, 0)));
        assert_eq!(next("0 0 * * *", occurrence - 1), Some(occurrence));
    }

    /// Unit case (private helper with real branching): occurrences never fall
    /// outside the JavaScript `Date` range, and both edges are representable.
    #[test]
    fn bounds_occurrences_to_the_date_range() {
        const MAX: i64 = 8_640_000_000_000_000;
        // The last representable instant has no strict successor.
        assert_eq!(next("* * * * *", MAX), None);
        // One minute earlier, the last instant is itself the next occurrence.
        assert_eq!(next("* * * * *", MAX - 1), Some(MAX));
        // A later date would be outside the range, even if the field matches.
        assert_eq!(next("0 0 1 1 *", MAX - 1), None);
        // The lower edge is representable, and a value below it is clamped.
        assert_eq!(next("* * * * *", -MAX), Some(-MAX + 60_000));
        assert_eq!(next("* * * * *", -MAX - 1), Some(-MAX + 60_000));
    }

    /// Unit case (private helper with real branching): the wait rechecks at
    /// bounded intervals and never sleeps the whole remaining delay at once.
    #[test]
    fn wait_slices_are_bounded_and_stop_at_the_due_time() {
        assert_eq!(wait_slice_ms(0.0, 0), None);
        assert_eq!(wait_slice_ms(1.0, 0), None);
        assert_eq!(wait_slice_ms(0.0, 1), Some(1));
        assert_eq!(wait_slice_ms(0.0, 60_000), Some(60_000));
        assert_eq!(wait_slice_ms(0.0, 60_001), Some(MAX_WAIT_SLICE_MS));
        assert_eq!(wait_slice_ms(-1.5, 0), Some(2));
    }

    /// Unit case (private helper with real branching): a forward wall-clock
    /// jump is noticed at the next bounded slice, not after the original delay.
    #[tokio::test]
    async fn wait_rechecks_a_forward_clock_jump() {
        let now = Rc::new(Cell::new(1_000.0));
        let clock = Rc::clone(&now);
        let sleeper = Rc::clone(&now);
        let slices = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&slices);
        let cancel = CancelState::new();
        let resolved = wait_until_with(
            301_000,
            &cancel,
            move || clock.get(),
            move |milliseconds| {
                recorded.borrow_mut().push(milliseconds);
                // An NTP step past the due time during the first slice.
                sleeper.set(sleeper.get() + 400_000.0);
                std::future::ready(())
            },
        )
        .await;
        assert!(resolved);
        assert_eq!(*slices.borrow(), vec![MAX_WAIT_SLICE_MS]);
        assert!(now.get() >= 301_000.0);
    }

    /// Unit case (private helper with real branching): a backward wall-clock
    /// jump extends the wait instead of resolving it before the due time.
    #[tokio::test]
    async fn wait_does_not_resolve_early_after_a_backward_clock_jump() {
        let now = Rc::new(Cell::new(1_000.0));
        let clock = Rc::clone(&now);
        let sleeper = Rc::clone(&now);
        let slices = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&slices);
        let mut first = true;
        let cancel = CancelState::new();
        let resolved = wait_until_with(
            301_000,
            &cancel,
            move || clock.get(),
            move |milliseconds| {
                recorded.borrow_mut().push(milliseconds);
                let mut value = sleeper.get();
                if first {
                    first = false;
                    // The clock steps backward while the first slice sleeps.
                    value -= 120_000.0;
                }
                sleeper.set(value + milliseconds as f64);
                std::future::ready(())
            },
        )
        .await;
        assert!(resolved);
        // The old single sleep resolved here with a clock before the due time.
        assert!(now.get() >= 301_000.0);
        assert!(
            slices.borrow().len() > 1,
            "the wait must recheck after the jump: {:?}",
            slices.borrow()
        );
        assert_eq!(slices.borrow()[0], MAX_WAIT_SLICE_MS);
    }
}
