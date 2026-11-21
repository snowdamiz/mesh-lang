/// Generates the catalog from its one table (`catalog.rs`): the closed
/// [`Code`](crate::Code) enum, `CATALOG` (a row per code, in `Code` order),
/// and `RETIRED`.
///
/// Three guarantees are compile errors rather than tests:
/// - The private `Ids` enum has a variant per ID, retired ones included, so a
///   duplicated or reused ID is rustc's E0428.
/// - `title`, `summary`, and `repair` are required by the grammar, and a
///   `const` assertion rejects an empty one.
/// - `Code` and `CATALOG` come from the same repetition, so `CATALOG[code as
///   usize]` is that code's row.
macro_rules! catalog {
    (@status) => { $crate::code::Status::Active };
    (@status transitional) => { $crate::code::Status::Transitional };
    (@topic) => { None };
    (@topic $topic:literal) => { Some($topic) };
    (@example) => { None };
    (@example $bad:literal $fixed:literal) => {
        Some($crate::code::Example { bad: $bad, fixed: $fixed })
    };
    (
        retired: [$($retired:ident),* $(,)?],
        $(
            $variant:ident = $id:ident $name:literal $severity:ident $($status:ident)? {
                title: $title:literal,
                summary: $summary:literal,
                repair: $repair:literal,
                $(topic: $topic:literal,)?
                $(example: ($bad:literal, $fixed:literal $(,)?),)?
            },
        )*
    ) => {
        /// A Mesh diagnostic code. Closed: only `catalog.rs` defines codes, so
        /// a code outside the catalog cannot be constructed, emitted, or
        /// documented. `#[non_exhaustive]` keeps per-code policy out of other
        /// crates' `match`es; it belongs in the catalog row.
        #[non_exhaustive]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub enum Code {
            $($variant,)*
        }

        #[allow(dead_code)]
        enum Ids {
            $($id,)*
            $($retired,)*
        }

        const _: () = {
            $(
                assert!(!$title.is_empty(), concat!(stringify!($id), " has an empty title"));
                assert!(!$summary.is_empty(), concat!(stringify!($id), " has an empty summary"));
                assert!(!$repair.is_empty(), concat!(stringify!($id), " has an empty repair"));
            )*
        };

        impl Code {
            pub(crate) const ALL: &'static [Code] = &[$(Code::$variant,)*];
        }

        pub(crate) const CATALOG: &[$crate::code::CodeInfo] = &[
            $(
                $crate::code::CodeInfo {
                    id: stringify!($id),
                    name: $name,
                    severity: $crate::code::Severity::$severity,
                    title: $title,
                    summary: $summary,
                    repair: $repair,
                    topic: catalog!(@topic $($topic)?),
                    example: catalog!(@example $($bad $fixed)?),
                    status: catalog!(@status $($status)?),
                },
            )*
        ];

        pub(crate) const RETIRED: &[&str] = &[$(stringify!($retired),)*];
    };
}
