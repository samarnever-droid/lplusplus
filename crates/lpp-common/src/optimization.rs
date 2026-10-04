use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptimizationLevel {
    O0,
    O1,
    O2,
    O3,
    Os,
    Oz,
}

impl OptimizationLevel {
    #[must_use]
    pub const fn budget(self) -> OptimizationBudget {
        match self {
            Self::O0 => OptimizationBudget {
                inline_cost: 0,
                optional_specialization_instances: 0,
                loop_unroll_factor: 0,
                fixed_point_iterations: 1,
            },
            Self::O1 => OptimizationBudget {
                inline_cost: 24,
                optional_specialization_instances: 32,
                loop_unroll_factor: 0,
                fixed_point_iterations: 2,
            },
            Self::O2 => OptimizationBudget {
                inline_cost: 80,
                optional_specialization_instances: 256,
                loop_unroll_factor: 4,
                fixed_point_iterations: 4,
            },
            Self::O3 => OptimizationBudget {
                inline_cost: 180,
                optional_specialization_instances: 1_024,
                loop_unroll_factor: 8,
                fixed_point_iterations: 6,
            },
            Self::Os => OptimizationBudget {
                inline_cost: 18,
                optional_specialization_instances: 96,
                loop_unroll_factor: 0,
                fixed_point_iterations: 3,
            },
            Self::Oz => OptimizationBudget {
                inline_cost: 4,
                optional_specialization_instances: 32,
                loop_unroll_factor: 0,
                fixed_point_iterations: 2,
            },
        }
    }

    #[must_use]
    pub const fn optimizes_for_size(self) -> bool {
        matches!(self, Self::Os | Self::Oz)
    }

    #[must_use]
    pub const fn enables_interprocedural(self) -> bool {
        matches!(self, Self::O2 | Self::O3 | Self::Os | Self::Oz)
    }
}

impl FromStr for OptimizationLevel {
    type Err = InvalidOptimizationLevel;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "0" | "O0" | "none" => Ok(Self::O0),
            "1" | "O1" | "basic" => Ok(Self::O1),
            "2" | "O2" | "speed" => Ok(Self::O2),
            "3" | "O3" | "aggressive" => Ok(Self::O3),
            "s" | "Os" | "size" => Ok(Self::Os),
            "z" | "Oz" | "min-size" => Ok(Self::Oz),
            _ => Err(InvalidOptimizationLevel(value.to_owned())),
        }
    }
}

impl fmt::Display for OptimizationLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::O0 => "O0",
            Self::O1 => "O1",
            Self::O2 => "O2",
            Self::O3 => "O3",
            Self::Os => "Os",
            Self::Oz => "Oz",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationBudget {
    pub inline_cost: u32,
    pub optional_specialization_instances: u32,
    pub loop_unroll_factor: u8,
    pub fixed_point_iterations: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationOptions {
    pub level: OptimizationLevel,
    pub verify_each_pass: bool,
}

impl OptimizationOptions {
    #[must_use]
    pub const fn development() -> Self {
        Self {
            level: OptimizationLevel::O1,
            verify_each_pass: true,
        }
    }

    #[must_use]
    pub const fn release() -> Self {
        Self {
            level: OptimizationLevel::O2,
            verify_each_pass: false,
        }
    }

    #[must_use]
    pub const fn budget(self) -> OptimizationBudget {
        self.level.budget()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidOptimizationLevel(String);

impl fmt::Display for InvalidOptimizationLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown optimization level '{}'; expected O0, O1, O2, O3, Os, or Oz",
            self.0
        )
    }
}

impl std::error::Error for InvalidOptimizationLevel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profiles_and_assigns_bounded_growth() {
        assert_eq!("O0".parse(), Ok(OptimizationLevel::O0));
        assert_eq!("speed".parse(), Ok(OptimizationLevel::O2));
        assert_eq!("Oz".parse(), Ok(OptimizationLevel::Oz));
        assert!("fastest".parse::<OptimizationLevel>().is_err());

        let aggressive = OptimizationLevel::O3.budget();
        assert!(aggressive.inline_cost > OptimizationLevel::O2.budget().inline_cost);
        assert!(aggressive.optional_specialization_instances <= 1_024);
        assert!(aggressive.fixed_point_iterations <= 6);
    }

    #[test]
    fn size_profiles_disable_growth_heavy_loop_unrolling() {
        for level in [OptimizationLevel::Os, OptimizationLevel::Oz] {
            assert!(level.optimizes_for_size());
            assert_eq!(level.budget().loop_unroll_factor, 0);
        }
    }
}
