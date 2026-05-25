//! Protocol `SField` registry ported from `xrpl/protocol/SField.*`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum SerializedTypeId {
    Unknown = -2,
    NotPresent = 0,
    UInt16 = 1,
    UInt32 = 2,
    UInt64 = 3,
    UInt128 = 4,
    UInt256 = 5,
    Amount = 6,
    VariableLength = 7,
    Account = 8,
    Number = 9,
    Int32 = 10,
    Int64 = 11,
    Object = 14,
    Array = 15,
    UInt8 = 16,
    UInt160 = 17,
    PathSet = 18,
    Vector256 = 19,
    UInt96 = 20,
    UInt192 = 21,
    UInt384 = 22,
    UInt512 = 23,
    Issue = 24,
    XChainBridge = 25,
    Currency = 26,
    Transaction = 10001,
    LedgerEntry = 10002,
    Validation = 10003,
    Metadata = 10004,
}

impl SerializedTypeId {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            -2 => Some(Self::Unknown),
            0 => Some(Self::NotPresent),
            1 => Some(Self::UInt16),
            2 => Some(Self::UInt32),
            3 => Some(Self::UInt64),
            4 => Some(Self::UInt128),
            5 => Some(Self::UInt256),
            6 => Some(Self::Amount),
            7 => Some(Self::VariableLength),
            8 => Some(Self::Account),
            9 => Some(Self::Number),
            10 => Some(Self::Int32),
            11 => Some(Self::Int64),
            14 => Some(Self::Object),
            15 => Some(Self::Array),
            16 => Some(Self::UInt8),
            17 => Some(Self::UInt160),
            18 => Some(Self::PathSet),
            19 => Some(Self::Vector256),
            20 => Some(Self::UInt96),
            21 => Some(Self::UInt192),
            22 => Some(Self::UInt384),
            23 => Some(Self::UInt512),
            24 => Some(Self::Issue),
            25 => Some(Self::XChainBridge),
            26 => Some(Self::Currency),
            10001 => Some(Self::Transaction),
            10002 => Some(Self::LedgerEntry),
            10003 => Some(Self::Validation),
            10004 => Some(Self::Metadata),
            _ => None,
        }
    }
}

pub const SERIALIZED_TYPE_NAME_MAP: &[(&str, i32)] = &[
    ("STI_UNKNOWN", SerializedTypeId::Unknown as i32),
    ("STI_NOTPRESENT", SerializedTypeId::NotPresent as i32),
    ("STI_UINT16", SerializedTypeId::UInt16 as i32),
    ("STI_UINT32", SerializedTypeId::UInt32 as i32),
    ("STI_UINT64", SerializedTypeId::UInt64 as i32),
    ("STI_UINT128", SerializedTypeId::UInt128 as i32),
    ("STI_UINT256", SerializedTypeId::UInt256 as i32),
    ("STI_AMOUNT", SerializedTypeId::Amount as i32),
    ("STI_VL", SerializedTypeId::VariableLength as i32),
    ("STI_ACCOUNT", SerializedTypeId::Account as i32),
    ("STI_NUMBER", SerializedTypeId::Number as i32),
    ("STI_INT32", SerializedTypeId::Int32 as i32),
    ("STI_INT64", SerializedTypeId::Int64 as i32),
    ("STI_OBJECT", SerializedTypeId::Object as i32),
    ("STI_ARRAY", SerializedTypeId::Array as i32),
    ("STI_UINT8", SerializedTypeId::UInt8 as i32),
    ("STI_UINT160", SerializedTypeId::UInt160 as i32),
    ("STI_PATHSET", SerializedTypeId::PathSet as i32),
    ("STI_VECTOR256", SerializedTypeId::Vector256 as i32),
    ("STI_UINT96", SerializedTypeId::UInt96 as i32),
    ("STI_UINT192", SerializedTypeId::UInt192 as i32),
    ("STI_UINT384", SerializedTypeId::UInt384 as i32),
    ("STI_UINT512", SerializedTypeId::UInt512 as i32),
    ("STI_ISSUE", SerializedTypeId::Issue as i32),
    ("STI_XCHAIN_BRIDGE", SerializedTypeId::XChainBridge as i32),
    ("STI_CURRENCY", SerializedTypeId::Currency as i32),
    ("STI_TRANSACTION", SerializedTypeId::Transaction as i32),
    ("STI_LEDGERENTRY", SerializedTypeId::LedgerEntry as i32),
    ("STI_VALIDATION", SerializedTypeId::Validation as i32),
    ("STI_METADATA", SerializedTypeId::Metadata as i32),
];

pub const fn field_code(id: SerializedTypeId, index: i32) -> i32 {
    field_code_raw(id as i32, index)
}

pub const fn field_code_raw(id: i32, index: i32) -> i32 {
    (id << 16) | index
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsSigning {
    No,
    Yes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSFieldError {
    InvalidFieldCode {
        code: i32,
    },
    DuplicateCode {
        code: i32,
        existing_symbol: &'static str,
    },
    DuplicateName {
        name: String,
        existing_symbol: &'static str,
    },
    DuplicateSymbol {
        symbol_name: String,
        existing_name: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SField {
    field_code: i32,
    field_type: SerializedTypeId,
    field_value: i32,
    field_name: &'static str,
    field_meta: u32,
    field_num: usize,
    signing_field: IsSigning,
    symbol_name: &'static str,
}

impl SField {
    pub const S_MD_NEVER: u32 = 0x00;
    pub const S_MD_CHANGE_ORIG: u32 = 0x01;
    pub const S_MD_CHANGE_NEW: u32 = 0x02;
    pub const S_MD_DELETE_FINAL: u32 = 0x04;
    pub const S_MD_CREATE: u32 = 0x08;
    pub const S_MD_ALWAYS: u32 = 0x10;
    pub const S_MD_BASE_TEN: u32 = 0x20;
    pub const S_MD_PSEUDO_ACCOUNT: u32 = 0x40;
    pub const S_MD_NEEDS_ASSET: u32 = 0x80;
    pub const S_MD_DEFAULT: u32 = Self::S_MD_CHANGE_ORIG
        | Self::S_MD_CHANGE_NEW
        | Self::S_MD_DELETE_FINAL
        | Self::S_MD_CREATE;

    pub const fn code(self) -> i32 {
        self.field_code
    }

    pub const fn field_type(self) -> SerializedTypeId {
        self.field_type
    }

    pub const fn field_value(self) -> i32 {
        self.field_value
    }

    pub const fn name(self) -> &'static str {
        self.field_name
    }

    pub const fn symbol_name(self) -> &'static str {
        self.symbol_name
    }

    pub const fn field_meta(self) -> u32 {
        self.field_meta
    }

    pub const fn field_num(self) -> usize {
        self.field_num
    }

    pub const fn signing_field(self) -> IsSigning {
        self.signing_field
    }

    pub const fn has_name(self) -> bool {
        self.field_code > 0
    }

    pub const fn is_invalid(self) -> bool {
        self.field_code == -1
    }

    pub const fn is_useful(self) -> bool {
        self.field_code > 0
    }

    pub const fn is_binary(self) -> bool {
        self.field_value < 256
    }

    pub const fn is_discardable(self) -> bool {
        self.field_value > 256
    }

    pub const fn should_meta(self, mask: u32) -> bool {
        (self.field_meta & mask) != 0
    }

    pub const fn should_include(self, with_signing_field: bool) -> bool {
        (self.field_value < 256)
            && (with_signing_field || matches!(self.signing_field, IsSigning::Yes))
    }

    pub fn compare(f1: Self, f2: Self) -> i32 {
        if f1.field_code <= 0 || f2.field_code <= 0 {
            return 0;
        }
        if f1.field_code < f2.field_code {
            -1
        } else if f1.field_code > f2.field_code {
            1
        } else {
            0
        }
    }
}

#[derive(Debug)]
pub(crate) struct SFieldSpec {
    symbol_name: &'static str,
    field_type: SerializedTypeId,
    field_value: i32,
    field_name: &'static str,
    field_meta: u32,
    signing: IsSigning,
    field_code_override: Option<i32>,
}

#[derive(Debug)]
struct SFieldRegistry {
    fields: Box<[SField]>,
    code_to_index: HashMap<i32, usize>,
    name_to_index: HashMap<&'static str, usize>,
    symbol_to_index: HashMap<&'static str, usize>,
}

impl SFieldRegistry {
    fn build() -> Self {
        let mut fields = Vec::with_capacity(SFIELD_SPECS.len());
        let mut code_to_index = HashMap::with_capacity(SFIELD_SPECS.len());
        let mut name_to_index = HashMap::with_capacity(SFIELD_SPECS.len());
        let mut symbol_to_index = HashMap::with_capacity(SFIELD_SPECS.len());

        for (index, spec) in SFIELD_SPECS.iter().enumerate() {
            let field = SField {
                field_code: spec
                    .field_code_override
                    .unwrap_or_else(|| field_code(spec.field_type, spec.field_value)),
                field_type: spec.field_type,
                field_value: spec.field_value,
                field_name: spec.field_name,
                field_meta: spec.field_meta,
                field_num: index + 1,
                signing_field: spec.signing,
                symbol_name: spec.symbol_name,
            };

            let previous_code = code_to_index.insert(field.field_code, index);
            assert!(
                previous_code.is_none(),
                "duplicate SField code {}",
                field.field_code
            );

            let previous_symbol = symbol_to_index.insert(field.symbol_name, index);
            assert!(
                previous_symbol.is_none(),
                "duplicate SField symbol {}",
                field.symbol_name
            );

            let previous_name = name_to_index.insert(field.field_name, index);
            assert!(
                previous_name.is_none(),
                "duplicate SField name {}",
                field.field_name
            );

            fields.push(field);
        }

        Self {
            fields: fields.into_boxed_slice(),
            code_to_index,
            name_to_index,
            symbol_to_index,
        }
    }

    fn by_index(&'static self, index: usize) -> &'static SField {
        &self.fields[index]
    }

    fn by_code(&'static self, code: i32) -> Option<&'static SField> {
        self.code_to_index
            .get(&code)
            .map(|index| self.by_index(*index))
    }

    fn by_name(&'static self, name: &str) -> Option<&'static SField> {
        self.name_to_index
            .get(name)
            .map(|index| self.by_index(*index))
    }

    fn by_symbol(&'static self, symbol_name: &str) -> Option<&'static SField> {
        self.symbol_to_index
            .get(symbol_name)
            .map(|index| self.by_index(*index))
    }
}

#[derive(Debug, Default)]
struct RuntimeSFieldRegistry {
    fields: Vec<&'static SField>,
    code_to_index: HashMap<i32, usize>,
    name_to_index: HashMap<&'static str, usize>,
    symbol_to_index: HashMap<&'static str, usize>,
}

impl RuntimeSFieldRegistry {
    fn by_code(&self, code: i32) -> Option<&'static SField> {
        self.code_to_index
            .get(&code)
            .map(|index| self.fields[*index])
    }

    fn by_name(&self, name: &str) -> Option<&'static SField> {
        self.name_to_index
            .get(name)
            .map(|index| self.fields[*index])
    }

    fn by_symbol(&self, symbol_name: &str) -> Option<&'static SField> {
        self.symbol_to_index
            .get(symbol_name)
            .map(|index| self.fields[*index])
    }
}

static SFIELD_REGISTRY: OnceLock<SFieldRegistry> = OnceLock::new();
static RUNTIME_SFIELD_REGISTRY: OnceLock<RwLock<RuntimeSFieldRegistry>> = OnceLock::new();

fn registry() -> &'static SFieldRegistry {
    SFIELD_REGISTRY.get_or_init(SFieldRegistry::build)
}

fn runtime_registry() -> &'static RwLock<RuntimeSFieldRegistry> {
    RUNTIME_SFIELD_REGISTRY.get_or_init(|| RwLock::new(RuntimeSFieldRegistry::default()))
}

pub fn all_sfields() -> &'static [SField] {
    &registry().fields
}

pub fn max_sfield_num() -> usize {
    let runtime_fields = runtime_registry()
        .read()
        .expect("runtime SField registry lock should not be poisoned")
        .fields
        .len();
    registry().fields.len() + runtime_fields
}

pub fn serialized_type_name_map() -> &'static BTreeMap<&'static str, i32> {
    static MAP: OnceLock<BTreeMap<&'static str, i32>> = OnceLock::new();
    MAP.get_or_init(|| SERIALIZED_TYPE_NAME_MAP.iter().copied().collect())
}

pub fn serialized_type_id_by_name(name: &str) -> Option<SerializedTypeId> {
    serialized_type_name_map()
        .get(name)
        .and_then(|value| SerializedTypeId::from_i32(*value))
}

pub fn get_field(code: i32) -> &'static SField {
    registry()
        .by_code(code)
        .or_else(|| {
            runtime_registry()
                .read()
                .expect("runtime SField registry lock should not be poisoned")
                .by_code(code)
        })
        .unwrap_or_else(sf_invalid)
}

pub fn get_field_by_name(name: &str) -> &'static SField {
    registry()
        .by_name(name)
        .or_else(|| {
            runtime_registry()
                .read()
                .expect("runtime SField registry lock should not be poisoned")
                .by_name(name)
        })
        .unwrap_or_else(sf_invalid)
}

pub fn get_field_by_symbol(symbol_name: &str) -> &'static SField {
    registry()
        .by_symbol(symbol_name)
        .or_else(|| {
            runtime_registry()
                .read()
                .expect("runtime SField registry lock should not be poisoned")
                .by_symbol(symbol_name)
        })
        .unwrap_or_else(sf_invalid)
}

pub fn register_runtime_sfield(
    symbol_name: &str,
    field_type: SerializedTypeId,
    field_value: i32,
    field_name: &str,
    field_meta: u32,
    signing: IsSigning,
    field_code_override: Option<i32>,
) -> Result<&'static SField, RuntimeSFieldError> {
    let field_code = field_code_override.unwrap_or_else(|| field_code(field_type, field_value));
    if field_code <= 0 {
        return Err(RuntimeSFieldError::InvalidFieldCode { code: field_code });
    }

    if let Some(existing) = registry().by_code(field_code) {
        return Err(RuntimeSFieldError::DuplicateCode {
            code: field_code,
            existing_symbol: existing.symbol_name(),
        });
    }
    if let Some(existing) = registry().by_name(field_name) {
        return Err(RuntimeSFieldError::DuplicateName {
            name: field_name.to_owned(),
            existing_symbol: existing.symbol_name(),
        });
    }
    if let Some(existing) = registry().by_symbol(symbol_name) {
        return Err(RuntimeSFieldError::DuplicateSymbol {
            symbol_name: symbol_name.to_owned(),
            existing_name: existing.name(),
        });
    }

    let mut runtime = runtime_registry()
        .write()
        .expect("runtime SField registry lock should not be poisoned");

    if let Some(existing) = runtime.by_code(field_code) {
        return Err(RuntimeSFieldError::DuplicateCode {
            code: field_code,
            existing_symbol: existing.symbol_name(),
        });
    }
    if let Some(existing) = runtime.by_name(field_name) {
        return Err(RuntimeSFieldError::DuplicateName {
            name: field_name.to_owned(),
            existing_symbol: existing.symbol_name(),
        });
    }
    if let Some(existing) = runtime.by_symbol(symbol_name) {
        return Err(RuntimeSFieldError::DuplicateSymbol {
            symbol_name: symbol_name.to_owned(),
            existing_name: existing.name(),
        });
    }

    let symbol_name = Box::leak(symbol_name.to_owned().into_boxed_str());
    let field_name = Box::leak(field_name.to_owned().into_boxed_str());
    let field_num = registry().fields.len() + runtime.fields.len() + 1;
    let field = Box::leak(Box::new(SField {
        field_code,
        field_type,
        field_value,
        field_name,
        field_meta,
        field_num,
        signing_field: signing,
        symbol_name,
    }));

    let runtime_index = runtime.fields.len();
    runtime.fields.push(field);
    runtime.code_to_index.insert(field_code, runtime_index);
    runtime.name_to_index.insert(field_name, runtime_index);
    runtime.symbol_to_index.insert(symbol_name, runtime_index);

    Ok(field)
}

pub fn sf_invalid() -> &'static SField {
    &registry().fields[0]
}

pub fn sf_generic() -> &'static SField {
    &registry().fields[1]
}

include!("generated_sfield_specs.rs");
