//! Durable identities for definitions, parameters, options, and behaviors.
//!
//! Names are editable labels. These identifiers stay stable across rename,
//! save/reopen, and undo. Copying an INSERT allocates a new `EntityId` and
//! copies parameter values; making a definition unique allocates a new
//! `BlockDefinitionId` and remaps every dependent binding.

use std::fmt;

// ------------------------------------------------------------
// Type: BlockDefinitionId
// Purpose: Identifies a block definition independently of its name.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockDefinitionId(pub u64);

impl BlockDefinitionId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for BlockDefinitionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "def:{}", self.0)
    }
}

// ------------------------------------------------------------
// Type: ParameterId
// Purpose: Keeps bindings valid after a parameter is renamed.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ParameterId(pub u64);

impl ParameterId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ParameterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "param:{}", self.0)
    }
}

// ------------------------------------------------------------
// Type: OptionId
// Purpose: Keeps choices valid after their labels change.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OptionId(pub u64);

impl OptionId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for OptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "opt:{}", self.0)
    }
}

// ------------------------------------------------------------
// Type: ActionId
// Purpose: Identifies a behavior for editing, history, and diagnostics.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ActionId(pub u64);

impl ActionId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "action:{}", self.0)
    }
}

// ------------------------------------------------------------
// Type: VertexId
// Purpose: Durable identity for a polyline vertex. Vector indices
//          are not stable across insert, delete, or reorder.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VertexId(pub u64);

impl VertexId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for VertexId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vtx:{}", self.0)
    }
}

// ------------------------------------------------------------
// Type: AnchorId
// Purpose: Stable destination for option-driven placement.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AnchorId(pub u64);

impl AnchorId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AnchorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "anchor:{}", self.0)
    }
}

// ------------------------------------------------------------
// Type: PresetId
// Purpose: Named complete configuration within one definition.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PresetId(pub u64);

impl PresetId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "preset:{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unassigned_is_zero_and_not_assigned() {
        assert!(!BlockDefinitionId::UNASSIGNED.is_assigned());
        assert!(!ParameterId::UNASSIGNED.is_assigned());
        assert!(!OptionId::UNASSIGNED.is_assigned());
        assert!(!ActionId::UNASSIGNED.is_assigned());
        assert!(!VertexId::UNASSIGNED.is_assigned());
        assert!(!AnchorId::UNASSIGNED.is_assigned());
        assert!(!PresetId::UNASSIGNED.is_assigned());
    }

    #[test]
    fn assigned_ids_compare_by_raw_value() {
        assert!(ParameterId(2) > ParameterId(1));
        assert_eq!(ActionId(9).raw(), 9);
    }
}
