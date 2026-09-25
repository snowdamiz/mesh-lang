//! Rowan-based concrete syntax tree types for Mesh.
//!
//! Defines the `MeshLanguage` marker type that connects [`SyntaxKind`] to
//! rowan's generic tree infrastructure, plus type aliases for convenience.

use crate::syntax_kind::SyntaxKind;

/// Marker type for Mesh's language in rowan's generic tree system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeshLanguage {}

impl rowan::Language for MeshLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        // Safety: SyntaxKind is #[repr(u16)] and all u16 values within the
        // enum's range are valid variants. Rowan only stores kinds that we
        // previously gave it via kind_to_raw.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

/// A CST node (interior node with children).
pub type SyntaxNode = rowan::SyntaxNode<MeshLanguage>;

/// A CST token (leaf node with text).
pub type SyntaxToken = rowan::SyntaxToken<MeshLanguage>;

/// Either a node or a token in the CST.
pub type SyntaxElement = rowan::SyntaxElement<MeshLanguage>;

#[cfg(test)]
mod tests {
    use super::*;
    use rowan::Language;

    #[test]
    fn kinds_round_trip_through_rowan() {
        for kind in [SyntaxKind::FN_KW, SyntaxKind::SOURCE_FILE, SyntaxKind::EOF] {
            assert_eq!(
                MeshLanguage::kind_from_raw(MeshLanguage::kind_to_raw(kind)),
                kind
            );
        }
    }
}
