//! Identity newtypes.
//!
//! The domain identifies its actors and aggregates by opaque string ids rather
//! than raw [`String`]s, so the compiler stops you mixing a user id up with a
//! report id (or a chunk of free-form text). They derive [`Eq`]/[`Hash`] so they
//! work as map keys and as the subject of authorization checks.

/// Stable identity of a [`User`](crate::models::users::User).
///
/// Distinct from the login username carried in
/// [`Credentials`](crate::models::auth::Credentials): the username can change,
/// the id does not.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(String);

impl UserId {
    /// Wrap an existing identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for UserId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for UserId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Stable identity of a department.
///
/// Departments are a fixed set defined at startup (see
/// [`DepartmentRegistry`](crate::models::department::DepartmentRegistry)). Users
/// are *manually* assigned to one; the assignment is what makes a user active and
/// what scopes a trainee's report queue to the signers in their department.
/// Carried by every effective [`Role`](crate::models::users::Role).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DepartmentId(String);

impl DepartmentId {
    /// Wrap an existing identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for DepartmentId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DepartmentId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Stable identity of a [`Report`](crate::models::report::Report).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReportId(String);

impl ReportId {
    /// Wrap an existing identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ReportId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ReportId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
