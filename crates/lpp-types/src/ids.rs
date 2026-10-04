use std::fmt;
use std::num::NonZeroU32;

macro_rules! compact_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            #[must_use]
            pub const fn from_raw(raw: u32) -> Self {
                assert!(raw < u32::MAX, "compact type ID index exceeds its range");
                Self(NonZeroU32::new(raw + 1).expect("index + 1 is nonzero"))
            }

            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0.get() - 1
            }

            #[must_use]
            pub(crate) fn from_index(index: usize) -> Option<Self> {
                let raw = u32::try_from(index).ok()?;
                (raw < u32::MAX).then(|| Self::from_raw(raw))
            }

            #[must_use]
            pub(crate) const fn index(self) -> usize {
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

compact_id!(TypeId);
compact_id!(TypeListId);
compact_id!(InferVarId);
compact_id!(TraitImplId);
