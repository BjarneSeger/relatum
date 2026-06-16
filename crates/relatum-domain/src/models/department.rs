//! The fixed set of departments an instance knows about.
//!
//! Departments are *hard-coded at startup*: the server reads them from config and
//! builds a [`DepartmentRegistry`], which the
//! [`UserAdmin`](crate::services::admin::UserAdmin) service consults to reject any
//! manual assignment to a department the instance does not recognise. The domain
//! never invents a department; it only validates against this set.

use crate::models::ids::DepartmentId;

/// The set of departments configured for this instance.
///
/// Order is irrelevant and duplicates are collapsed; lookups are membership
/// tests, so the backing store is a deduplicated list.
#[derive(Debug, Clone, Default)]
pub struct DepartmentRegistry {
    departments: Vec<DepartmentId>,
}

impl DepartmentRegistry {
    /// Build a registry from the departments configured at startup, dropping
    /// duplicates while preserving first-seen order.
    pub fn new(departments: impl IntoIterator<Item = DepartmentId>) -> Self {
        let mut deduped: Vec<DepartmentId> = Vec::new();
        for department in departments {
            if !deduped.contains(&department) {
                deduped.push(department);
            }
        }
        Self {
            departments: deduped,
        }
    }

    /// Whether `department` is one of the configured departments.
    pub fn contains(&self, department: &DepartmentId) -> bool {
        self.departments.contains(department)
    }

    /// Every configured department.
    pub fn all(&self) -> &[DepartmentId] {
        &self.departments
    }

    /// Whether no departments are configured at all.
    pub fn is_empty(&self) -> bool {
        self.departments.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_reflects_configured_departments() {
        let registry =
            DepartmentRegistry::new([DepartmentId::new("blue"), DepartmentId::new("red")]);
        assert!(registry.contains(&DepartmentId::new("blue")));
        assert!(registry.contains(&DepartmentId::new("red")));
        assert!(!registry.contains(&DepartmentId::new("green")));
    }

    #[test]
    fn duplicates_are_collapsed_preserving_order() {
        let registry = DepartmentRegistry::new([
            DepartmentId::new("blue"),
            DepartmentId::new("blue"),
            DepartmentId::new("red"),
        ]);
        assert_eq!(registry.all().len(), 2);
        assert_eq!(registry.all()[0].as_str(), "blue");
        assert_eq!(registry.all()[1].as_str(), "red");
    }

    #[test]
    fn empty_registry_contains_nothing() {
        let registry = DepartmentRegistry::default();
        assert!(registry.is_empty());
        assert!(!registry.contains(&DepartmentId::new("blue")));
    }
}
