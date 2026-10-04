use std::fmt;
use std::num::NonZeroU32;

pub trait ArenaId: Copy + Eq {
    fn from_index(index: usize) -> Option<Self>;
    fn index(self) -> usize;
}

macro_rules! compact_id {
    ($name:ident) => {
        /// Compact arena identity. The stored value is index + 1 so
        /// `Option<$name>` retains the same four-byte representation.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            #[must_use]
            pub const fn from_raw(raw: u32) -> Self {
                assert!(raw < u32::MAX, "compact ID index exceeds its range");
                Self(NonZeroU32::new(raw + 1).expect("index + 1 is nonzero"))
            }

            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0.get() - 1
            }
        }

        impl ArenaId for $name {
            fn from_index(index: usize) -> Option<Self> {
                let raw = u32::try_from(index).ok()?;
                (raw < u32::MAX).then(|| Self::from_raw(raw))
            }

            fn index(self) -> usize {
                self.raw() as usize
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.raw())
            }
        }
    };
}

compact_id!(Symbol);
compact_id!(DefId);
compact_id!(HirItemId);
compact_id!(BodyId);
compact_id!(OriginId);
compact_id!(ScopeId);
compact_id!(LocalId);
compact_id!(ExprId);
compact_id!(StmtId);
compact_id!(TypeRefId);
compact_id!(TypeParamId);
compact_id!(FieldId);
compact_id!(VariantId);
compact_id!(MatchArmId);
compact_id!(InstanceId);
