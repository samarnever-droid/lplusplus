use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbiRegistry {
    pub schema_version: u32,
    pub abi_version: u32,
    pub compatibility_source: String,
    #[serde(rename = "builtin")]
    pub builtins: Vec<Builtin>,
    #[serde(default, rename = "symbol_override")]
    pub symbol_overrides: Vec<SymbolOverride>,
    #[serde(default, rename = "builtin_name_override")]
    pub builtin_name_overrides: Vec<BuiltinNameOverride>,
}

impl AbiRegistry {
    pub fn parse(input: &str) -> Result<Self, SchemaError> {
        let registry: Self = toml::from_str(input).map_err(SchemaError::Toml)?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.schema_version != 1 {
            return Err(SchemaError::UnsupportedSchema(self.schema_version));
        }
        if self.abi_version == 0 {
            return Err(SchemaError::InvalidAbiVersion);
        }

        let duplicate_name_overrides = self
            .builtin_name_overrides
            .iter()
            .map(|item| item.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut names = BTreeMap::<&str, &Builtin>::new();
        for (index, builtin) in self.builtins.iter().enumerate() {
            if builtin.name.is_empty() {
                return Err(SchemaError::EmptyBuiltinName(index));
            }
            if let Some(previous) = names.insert(builtin.name.as_str(), builtin)
                && previous != builtin
                && !duplicate_name_overrides.contains(builtin.name.as_str())
            {
                return Err(SchemaError::DuplicateBuiltinName(builtin.name.clone()));
            }
            if builtin.parameter_ownership.len() != builtin.parameters.len() {
                return Err(SchemaError::OwnershipArity {
                    name: builtin.name.clone(),
                    parameters: builtin.parameters.len(),
                    ownership: builtin.parameter_ownership.len(),
                });
            }
        }

        let mut name_overrides = BTreeSet::new();
        for name_override in &self.builtin_name_overrides {
            if !name_overrides.insert(name_override.name.as_str()) {
                return Err(SchemaError::DuplicateNameOverride(
                    name_override.name.clone(),
                ));
            }
            if name_override.reason.trim().is_empty() {
                return Err(SchemaError::MissingNameOverrideReason(
                    name_override.name.clone(),
                ));
            }
        }

        let mut overrides = BTreeSet::new();
        for symbol_override in &self.symbol_overrides {
            if !overrides.insert(symbol_override.symbol.as_str()) {
                return Err(SchemaError::DuplicateOverride(
                    symbol_override.symbol.clone(),
                ));
            }
            if symbol_override.reason.trim().is_empty() {
                return Err(SchemaError::MissingOverrideReason(
                    symbol_override.symbol.clone(),
                ));
            }
        }

        for (symbol, signatures) in self.symbol_signatures() {
            if signatures.len() > 1 && !overrides.contains(symbol.as_str()) {
                return Err(SchemaError::ConflictingSymbol {
                    symbol,
                    signatures: signatures.into_iter().collect(),
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn symbols(&self) -> BTreeSet<&str> {
        self.builtins
            .iter()
            .filter_map(|builtin| (!builtin.symbol.is_empty()).then_some(builtin.symbol.as_str()))
            .collect()
    }

    #[must_use]
    pub fn runtime_symbols(&self) -> BTreeSet<&str> {
        self.symbols()
            .into_iter()
            .filter(|symbol| symbol.starts_with("lpp_"))
            .collect()
    }

    fn symbol_signatures(&self) -> BTreeMap<String, BTreeSet<LoweringSignature>> {
        let mut signatures = BTreeMap::<String, BTreeSet<LoweringSignature>>::new();
        for builtin in &self.builtins {
            if builtin.symbol.is_empty() {
                continue;
            }
            signatures
                .entry(builtin.symbol.clone())
                .or_default()
                .insert(LoweringSignature {
                    parameters: builtin.lowering_parameters.clone(),
                    result: builtin.lowering_result,
                });
        }
        signatures
    }

    pub(crate) fn canonical_signatures(&self) -> BTreeMap<&str, LoweringSignature> {
        let overrides = self
            .symbol_overrides
            .iter()
            .map(|item| {
                (
                    item.symbol.as_str(),
                    LoweringSignature {
                        parameters: item.parameters.clone(),
                        result: item.result,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut signatures = BTreeMap::new();
        for builtin in &self.builtins {
            if builtin.symbol.is_empty() || signatures.contains_key(builtin.symbol.as_str()) {
                continue;
            }
            let signature = overrides
                .get(builtin.symbol.as_str())
                .cloned()
                .unwrap_or_else(|| LoweringSignature {
                    parameters: builtin.lowering_parameters.clone(),
                    result: builtin.lowering_result,
                });
            signatures.insert(builtin.symbol.as_str(), signature);
        }
        signatures
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Builtin {
    pub name: String,
    pub symbol: String,
    pub feature: String,
    pub parameters: Vec<AbiType>,
    pub result: AbiType,
    /// The language-level result kind, when it differs from the ABI
    /// result. The v1 runtime returns `i64` 0/1 for its boolean
    /// predicates (`lt_u`, `str_contains`, ...), but the language
    /// treats the value as a `bool`; the semantic layer carries that
    /// split so the type stage and the codegen's result conversion
    /// agree with the oracle.
    #[serde(default)]
    pub semantic_result: Option<AbiType>,
    pub lowering_parameters: Vec<AbiType>,
    pub lowering_result: AbiType,
    pub parameter_ownership: Vec<Ownership>,
    pub result_ownership: Ownership,
    pub effects: Vec<String>,
    pub targets: Vec<TargetAvailability>,
}

impl Builtin {
    /// The semantic result kind: the override, or the ABI result.
    #[must_use]
    pub const fn semantic_result_type(&self) -> AbiType {
        match self.semantic_result {
            Some(kind) => kind,
            None => self.result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinNameOverride {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolOverride {
    pub symbol: String,
    pub parameters: Vec<AbiType>,
    pub result: AbiType,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbiType {
    Any,
    Bool,
    F64,
    I32,
    I64,
    Str,
    StrSlice,
    VectorI64x2,
    Void,
}

impl AbiType {
    pub(crate) const fn c_type(self) -> &'static str {
        match self {
            Self::Bool => "uint8_t",
            Self::F64 => "double",
            Self::I32 => "int32_t",
            Self::I64 | Self::Str | Self::StrSlice | Self::VectorI64x2 | Self::Any => "int64_t",
            Self::Void => "void",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Borrowed,
    Consumed,
    ReturnedOwned,
    LegacyUnspecified,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetAvailability {
    Legacy,
    Native,
    Wasm,
    Freestanding,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoweringSignature {
    pub parameters: Vec<AbiType>,
    pub result: AbiType,
}

impl fmt::Display for LoweringSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "(")?;
        for (index, parameter) in self.parameters.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{parameter:?}")?;
        }
        write!(formatter, ") -> {:?}", self.result)
    }
}

#[derive(Debug)]
pub enum SchemaError {
    Toml(toml::de::Error),
    UnsupportedSchema(u32),
    InvalidAbiVersion,
    EmptyBuiltinName(usize),
    DuplicateBuiltinName(String),
    DuplicateNameOverride(String),
    MissingNameOverrideReason(String),
    OwnershipArity {
        name: String,
        parameters: usize,
        ownership: usize,
    },
    DuplicateOverride(String),
    MissingOverrideReason(String),
    ConflictingSymbol {
        symbol: String,
        signatures: Vec<LoweringSignature>,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(error) => write!(formatter, "invalid ABI TOML: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported ABI schema version {version}")
            }
            Self::InvalidAbiVersion => formatter.write_str("ABI version must be greater than zero"),
            Self::EmptyBuiltinName(index) => write!(formatter, "builtin {index} has an empty name"),
            Self::DuplicateBuiltinName(name) => {
                write!(formatter, "duplicate builtin name '{name}'")
            }
            Self::DuplicateNameOverride(name) => {
                write!(formatter, "duplicate builtin-name override for '{name}'")
            }
            Self::MissingNameOverrideReason(name) => {
                write!(
                    formatter,
                    "builtin-name override for '{name}' has no reason"
                )
            }
            Self::OwnershipArity {
                name,
                parameters,
                ownership,
            } => write!(
                formatter,
                "builtin '{name}' has {parameters} parameters but {ownership} ownership entries"
            ),
            Self::DuplicateOverride(symbol) => {
                write!(formatter, "duplicate signature override for '{symbol}'")
            }
            Self::MissingOverrideReason(symbol) => {
                write!(formatter, "signature override for '{symbol}' has no reason")
            }
            Self::ConflictingSymbol { symbol, signatures } => {
                write!(formatter, "symbol '{symbol}' has conflicting signatures: ")?;
                for (index, signature) in signatures.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str("; ")?;
                    }
                    write!(formatter, "{signature}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SchemaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Toml(error) => Some(error),
            _ => None,
        }
    }
}
