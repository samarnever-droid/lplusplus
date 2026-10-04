use std::fmt;
use std::num::NonZeroU32;

use lpp_hir::ArenaId;

macro_rules! compact_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            #[must_use]
            pub const fn from_raw(raw: u32) -> Self {
                assert!(raw < u32::MAX, "compact MIR ID exceeds its range");
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

compact_id!(MirFunctionId);
compact_id!(MirAggregateId);
compact_id!(MirFieldId);
compact_id!(MirVariantId);
compact_id!(MirStringId);
compact_id!(BasicBlockId);
compact_id!(MirLocalId);
compact_id!(MirPlaceId);
compact_id!(InstructionId);
