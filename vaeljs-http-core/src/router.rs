//! Go 1.22+ `ServeMux`-style HTTP method and path-parameter router.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};

use crate::HttpCoreError;
use crate::policy::{is_http_token_char, percent_decode_segment};

/// Default maximum number of routes permitted in a single [`Router`].
pub const DEFAULT_MAX_ROUTES: usize = 4096;

/// Default maximum number of `/`-separated segments in a route pattern or request path.
pub const DEFAULT_MAX_PATH_SEGMENTS: usize = 128;

/// Default maximum byte length of a route pattern path or request path.
pub const DEFAULT_MAX_PATH_BYTES: usize = 8192;

/// Outcome of matching a request method and URL path in [`Router::match_route`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteMatch {
    /// A registered route matched both the request path and HTTP method.
    Matched {
        /// Opaque handler identifier supplied to [`Router::add`].
        handler_id: usize,
        /// Percent-decoded path parameters in route segment order.
        params: Vec<(String, String)>,
    },
    /// One or more routes matched the path, but none matched the method.
    MethodNotAllowed {
        /// Sorted, deduplicated list of allowed HTTP methods for this path.
        allow: Vec<String>,
    },
    /// No registered route matched the request path.
    NotFound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Segment {
    Literal(String),
    Param(String),
    Wildcard(String),
}

impl Segment {
    const fn rank(&self) -> u8 {
        match self {
            Self::Literal(_) => 3,
            Self::Param(_) => 2,
            Self::Wildcard(_) => 1,
        }
    }

    fn conflicts_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Literal(a), Self::Literal(b)) => a == b,
            (Self::Param(_), Self::Param(_)) | (Self::Wildcard(_), Self::Wildcard(_)) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum MethodRank {
    Any = 1,
    GetForHead = 2,
    Exact = 3,
}

struct CandidateMatch<'a> {
    route: &'a Route,
    method_rank: MethodRank,
    params: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct Route {
    method: Option<String>,
    segments: Vec<Segment>,
    handler_id: usize,
}

impl Route {
    fn conflicts_with(&self, other: &Self) -> bool {
        self.method == other.method
            && self.segments.len() == other.segments.len()
            && self
                .segments
                .iter()
                .zip(&other.segments)
                .all(|(a, b)| a.conflicts_with(b))
    }

    fn has_wildcard(&self) -> bool {
        matches!(self.segments.last(), Some(Segment::Wildcard(_)))
    }

    fn rank_at(&self, idx: usize) -> u8 {
        if idx < self.segments.len() {
            self.segments[idx].rank()
        } else {
            // Only reachable when the final segment is a wildcard.
            1
        }
    }

    fn match_path(&self, req_segs: &[String]) -> Option<Vec<(String, String)>> {
        if self.has_wildcard() {
            if req_segs.len() < self.segments.len() {
                return None;
            }
        } else if req_segs.len() != self.segments.len() {
            return None;
        }

        let mut params = Vec::new();
        for (idx, seg) in self.segments.iter().enumerate() {
            match seg {
                Segment::Literal(lit) => {
                    if req_segs[idx] != *lit {
                        return None;
                    }
                }
                Segment::Param(name) => {
                    if req_segs[idx].is_empty() {
                        return None;
                    }
                    params.push((name.clone(), req_segs[idx].clone()));
                }
                Segment::Wildcard(name) => {
                    let remainder = req_segs[idx..].join("/");
                    params.push((name.clone(), remainder));
                    break;
                }
            }
        }

        Some(params)
    }
}

fn compare_route_specificity(
    a: &Route,
    a_rank: MethodRank,
    b: &Route,
    b_rank: MethodRank,
    req_len: usize,
) -> Ordering {
    for idx in 0..req_len {
        let ord = a.rank_at(idx).cmp(&b.rank_at(idx));
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a_rank.cmp(&b_rank)
}

/// Deterministic HTTP method and path-parameter router.
///
/// Patterns follow Go 1.22+ `ServeMux` syntax without implicit subtree
/// redirection or ambiguity panics:
///
/// * Optional `"METHOD "` prefix where `METHOD` is a validated uppercase ASCII
///   HTTP token (for example `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`,
///   `OPTIONS`). When omitted, the route matches any valid HTTP method.
/// * Path starting with `/`, composed of:
///   * literal segments (`/users/me`, percent-decoded at registration),
///   * named single-segment parameters (`/users/{id}`), matching one non-empty
///     segment, or
///   * a trailing multi-segment wildcard (`/files/{path...}`), which must be
///     the final segment.
///
/// Precedence is deterministic: left-to-right segment specificity
/// (`literal > {param} > {wildcard...}`), then method-specific over any-method.
/// `GET` routes automatically satisfy `HEAD` requests unless an explicit `HEAD`
/// route matches with equal or higher specificity.
///
/// Bounded by [`DEFAULT_MAX_ROUTES`] (configurable via
/// [`Router::with_max_routes`]), [`DEFAULT_MAX_PATH_SEGMENTS`], and
/// [`DEFAULT_MAX_PATH_BYTES`].
///
/// # Examples
///
/// ```
/// use vaeljs_http_core::{RouteMatch, Router};
///
/// let mut router = Router::new();
/// router.add("GET /users/{id}", 1).unwrap();
/// router.add("GET /users/me", 2).unwrap();
/// router.add("/files/{path...}", 3).unwrap();
///
/// // Literal segments take precedence over parameters.
/// assert_eq!(
///     router.match_route("GET", "/users/me"),
///     RouteMatch::Matched {
///         handler_id: 2,
///         params: vec![],
///     }
/// );
///
/// // GET routes also handle HEAD automatically.
/// assert_eq!(
///     router.match_route("HEAD", "/users/alice%20smith"),
///     RouteMatch::Matched {
///         handler_id: 1,
///         params: vec![("id".to_owned(), "alice smith".to_owned())],
///     }
/// );
///
/// // Wrong method on a matched path yields 405 with a sorted Allow list.
/// assert_eq!(
///     router.match_route("POST", "/users/42"),
///     RouteMatch::MethodNotAllowed {
///         allow: vec!["GET".to_owned(), "HEAD".to_owned()],
///     }
/// );
/// ```
#[derive(Clone, Debug)]
pub struct Router {
    routes: Vec<Route>,
    max_routes: usize,
}

impl Default for Router {
    fn default() -> Self {
        Self::with_max_routes(DEFAULT_MAX_ROUTES)
    }
}

impl Router {
    /// Creates an empty [`Router`] bounded by [`DEFAULT_MAX_ROUTES`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty [`Router`] with an explicit maximum route capacity.
    #[must_use]
    pub fn with_max_routes(max_routes: usize) -> Self {
        Self {
            routes: Vec::new(),
            max_routes,
        }
    }

    /// Returns the number of registered routes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Returns `true` if no routes have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Registers a route pattern associated with `handler_id`.
    ///
    /// Literal segments are percent-decoded at registration so equivalent
    /// encoded and unencoded routes are normalized and conflict-checked.
    ///
    /// # Errors
    ///
    /// Returns [`HttpCoreError::RouteLimitExceeded`] if the router has reached
    /// its configured maximum route capacity, [`HttpCoreError::InvalidPattern`]
    /// if the pattern is empty, exceeds [`DEFAULT_MAX_PATH_BYTES`] or
    /// [`DEFAULT_MAX_PATH_SEGMENTS`], has an invalid HTTP method prefix, lacks a
    /// leading `/`, contains invalid segment/parameter syntax or invalid
    /// percent-encoding, places a `{wildcard...}` segment before the end, or
    /// repeats a parameter name, or [`HttpCoreError::RouteConflict`] if an
    /// equivalent route pattern for the same method scope is already
    /// registered.
    pub fn add(&mut self, pattern: &str, handler_id: usize) -> Result<(), HttpCoreError> {
        if self.routes.len() >= self.max_routes {
            return Err(HttpCoreError::RouteLimitExceeded {
                limit: self.max_routes,
            });
        }
        let route = parse_route(pattern, handler_id)?;
        if self
            .routes
            .iter()
            .any(|existing| existing.conflicts_with(&route))
        {
            return Err(HttpCoreError::RouteConflict {
                reason: "route pattern conflicts with an already-registered route",
            });
        }
        self.routes.push(route);
        Ok(())
    }

    /// Matches an HTTP `method` and request `path` against registered routes.
    ///
    /// Callers must strip any `?query` or `#fragment` suffix before calling this
    /// method; `path` is treated purely as the URL pathname.
    ///
    /// Matched `{param}` and `{wildcard...}` values are safely percent-decoded
    /// as UTF-8. Requests exceeding [`DEFAULT_MAX_PATH_BYTES`] or
    /// [`DEFAULT_MAX_PATH_SEGMENTS`], or containing invalid tokens, missing
    /// leading `/`, ASCII control bytes, malformed percent-escapes, `%00`, or
    /// invalid UTF-8 fail closed with [`RouteMatch::NotFound`].
    #[must_use]
    pub fn match_route(&self, method: &str, path: &str) -> RouteMatch {
        if !is_valid_request_method(method)
            || !path.starts_with('/')
            || path.len() > DEFAULT_MAX_PATH_BYTES
            || path.bytes().any(|b| b < 0x20 || b == 0x7F)
        {
            return RouteMatch::NotFound;
        }

        let mut req_segs = Vec::new();
        for raw_seg in path[1..].split('/') {
            if req_segs.len() >= DEFAULT_MAX_PATH_SEGMENTS {
                return RouteMatch::NotFound;
            }
            let Ok(decoded) = percent_decode_segment(raw_seg) else {
                return RouteMatch::NotFound;
            };
            req_segs.push(decoded);
        }

        let mut best: Option<CandidateMatch<'_>> = None;
        let mut path_matched = false;
        let mut allowed_methods = BTreeSet::new();

        for route in &self.routes {
            let Some(params) = route.match_path(&req_segs) else {
                continue;
            };
            path_matched = true;

            if let Some(route_method) = &route.method {
                allowed_methods.insert(route_method.clone());
                if route_method == "GET" {
                    allowed_methods.insert("HEAD".to_owned());
                }
            }

            let method_rank = match route.method.as_deref() {
                Some(m) if m == method => Some(MethodRank::Exact),
                Some("GET") if method == "HEAD" => Some(MethodRank::GetForHead),
                None => Some(MethodRank::Any),
                Some(_) => None,
            };

            if let Some(rank) = method_rank {
                let replace = best.as_ref().is_none_or(|current| {
                    compare_route_specificity(
                        route,
                        rank,
                        current.route,
                        current.method_rank,
                        req_segs.len(),
                    ) == Ordering::Greater
                });
                if replace {
                    best = Some(CandidateMatch {
                        route,
                        method_rank: rank,
                        params,
                    });
                }
            }
        }

        if let Some(matched) = best {
            RouteMatch::Matched {
                handler_id: matched.route.handler_id,
                params: matched.params,
            }
        } else if path_matched {
            RouteMatch::MethodNotAllowed {
                allow: allowed_methods.into_iter().collect(),
            }
        } else {
            RouteMatch::NotFound
        }
    }
}

fn is_valid_uppercase_method(method: &str) -> bool {
    !method.is_empty()
        && method
            .bytes()
            .all(|b| is_http_token_char(b) && !b.is_ascii_lowercase())
}

fn is_valid_request_method(method: &str) -> bool {
    !method.is_empty() && method.bytes().all(is_http_token_char)
}

fn is_valid_param_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn parse_route(pattern: &str, handler_id: usize) -> Result<Route, HttpCoreError> {
    if pattern.is_empty() {
        return Err(HttpCoreError::InvalidPattern {
            reason: "route pattern must not be empty",
        });
    }
    if pattern.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(HttpCoreError::InvalidPattern {
            reason: "route pattern must not contain control characters",
        });
    }

    let (method, raw_path) = if pattern.starts_with('/') {
        (None, pattern)
    } else {
        let Some((raw_method, raw_path)) = pattern.split_once(' ') else {
            return Err(HttpCoreError::InvalidPattern {
                reason: "route pattern path must start with '/'",
            });
        };
        if !is_valid_uppercase_method(raw_method) {
            return Err(HttpCoreError::InvalidPattern {
                reason: "route method prefix must be an uppercase ASCII HTTP token",
            });
        }
        (Some(raw_method.to_owned()), raw_path)
    };

    if !raw_path.starts_with('/') {
        return Err(HttpCoreError::InvalidPattern {
            reason: "route pattern path must start with '/'",
        });
    }
    if raw_path.len() > DEFAULT_MAX_PATH_BYTES {
        return Err(HttpCoreError::InvalidPattern {
            reason: "route pattern path exceeds maximum byte length",
        });
    }
    if raw_path
        .bytes()
        .any(|b| b == b' ' || b == b'?' || b == b'#' || b == b'\\')
    {
        return Err(HttpCoreError::InvalidPattern {
            reason: "route pattern path contains disallowed characters",
        });
    }

    let raw_segments: Vec<&str> = raw_path[1..].split('/').collect();
    let total = raw_segments.len();
    if total > DEFAULT_MAX_PATH_SEGMENTS {
        return Err(HttpCoreError::InvalidPattern {
            reason: "route pattern exceeds maximum path segment count",
        });
    }

    let mut segments = Vec::with_capacity(total);
    let mut seen_params = HashSet::new();

    for (idx, raw_seg) in raw_segments.into_iter().enumerate() {
        if raw_path != "/" && raw_seg.is_empty() && idx + 1 < total {
            return Err(HttpCoreError::InvalidPattern {
                reason: "route pattern must not contain empty interior segments",
            });
        }

        if raw_seg.contains('{') || raw_seg.contains('}') {
            if !(raw_seg.starts_with('{') && raw_seg.ends_with('}')) {
                return Err(HttpCoreError::InvalidPattern {
                    reason: "parameter syntax must span an entire path segment",
                });
            }
            let inner = &raw_seg[1..raw_seg.len() - 1];
            if inner.contains('{') || inner.contains('}') {
                return Err(HttpCoreError::InvalidPattern {
                    reason: "nested braces are not allowed in parameter segments",
                });
            }

            if let Some(name) = inner.strip_suffix("...") {
                if idx + 1 != total {
                    return Err(HttpCoreError::InvalidPattern {
                        reason: "wildcard parameter must be the last segment",
                    });
                }
                if !is_valid_param_name(name) {
                    return Err(HttpCoreError::InvalidPattern {
                        reason: "wildcard parameter name is invalid",
                    });
                }
                if !seen_params.insert(name.to_owned()) {
                    return Err(HttpCoreError::InvalidPattern {
                        reason: "duplicate parameter name in route pattern",
                    });
                }
                segments.push(Segment::Wildcard(name.to_owned()));
            } else {
                if !is_valid_param_name(inner) {
                    return Err(HttpCoreError::InvalidPattern {
                        reason: "parameter name is invalid",
                    });
                }
                if !seen_params.insert(inner.to_owned()) {
                    return Err(HttpCoreError::InvalidPattern {
                        reason: "duplicate parameter name in route pattern",
                    });
                }
                segments.push(Segment::Param(inner.to_owned()));
            }
        } else {
            let decoded = percent_decode_segment(raw_seg)
                .map_err(|reason| HttpCoreError::InvalidPattern { reason })?;
            segments.push(Segment::Literal(decoded));
        }
    }

    Ok(Route {
        method,
        segments,
        handler_id,
    })
}
