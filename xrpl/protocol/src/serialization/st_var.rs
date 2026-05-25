//! `detail::STVar` over the currently ported `ST*` values.
//!
//! This mirrors the reference storage policy more closely than the earlier enum
//! holder:
//! - values at or below the 72-byte small-object threshold stay inline,
//! - larger values fall back to heap allocation,
//! - and callers still interact through `&dyn StBase`.

use std::mem::{MaybeUninit, align_of, size_of};
use std::ptr;

use crate::{
    SField, STAccount, STAmount, STArray, STBlob, STCurrency, STInt32, STIssue, STNumber, STObject,
    STPathSet, STUInt8, STUInt16, STUInt32, STUInt64, STUInt128, STUInt160, STUInt192, STUInt256,
    STVector256, STXChainBridge, SerialIter, SerializedTypeId, StBase, StBaseCore, ValidationError,
};

const STVAR_INLINE_SIZE: usize = 72;
const STVAR_INLINE_ALIGN: usize = 16;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct InlineStorage {
    bytes: [MaybeUninit<u8>; STVAR_INLINE_SIZE],
}

impl InlineStorage {
    const fn uninit() -> Self {
        Self {
            bytes: [MaybeUninit::uninit(); STVAR_INLINE_SIZE],
        }
    }
}

union Storage {
    inline: InlineStorage,
    heap: *mut (),
}

struct TypeOps {
    clone_fn: unsafe fn(*const ()) -> STVar,
    drop_fn: unsafe fn(*mut (), bool),
    as_ref_fn: unsafe fn(*const ()) -> *const dyn StBase,
    as_mut_fn: unsafe fn(*mut ()) -> *mut dyn StBase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct STBaseValue {
    core: StBaseCore,
}

impl STBaseValue {
    fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
        }
    }
}

impl StBase for STBaseValue {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        &mut self.core
    }
}

fn fits_inline<T>() -> bool {
    size_of::<T>() <= STVAR_INLINE_SIZE && align_of::<T>() <= STVAR_INLINE_ALIGN
}

unsafe fn clone_impl<T>(src: *const ()) -> STVar
where
    T: StBase + Clone + Send + Sync + 'static,
{
    let value = unsafe { (&*(src.cast::<T>())).clone() };
    STVar::new(value)
}

unsafe fn drop_impl<T>(ptr: *mut (), inline: bool)
where
    T: StBase + Clone + Send + Sync + 'static,
{
    if inline {
        unsafe { ptr::drop_in_place(ptr.cast::<T>()) };
    } else {
        unsafe { drop(Box::from_raw(ptr.cast::<T>())) };
    }
}

unsafe fn as_ref_impl<T>(ptr: *const ()) -> *const dyn StBase
where
    T: StBase + Clone + Send + Sync + 'static,
{
    let typed: *const T = ptr.cast::<T>();
    typed as *const dyn StBase
}

unsafe fn as_mut_impl<T>(ptr: *mut ()) -> *mut dyn StBase
where
    T: StBase + Clone + Send + Sync + 'static,
{
    let typed: *mut T = ptr.cast::<T>();
    typed as *mut dyn StBase
}

fn type_ops<T>() -> &'static TypeOps
where
    T: StBase + Clone + Send + Sync + 'static,
{
    &TypeOpsHolder::<T>::OPS
}

struct TypeOpsHolder<T>(std::marker::PhantomData<T>);

impl<T> TypeOpsHolder<T>
where
    T: StBase + Clone + Send + Sync + 'static,
{
    const OPS: TypeOps = TypeOps {
        clone_fn: clone_impl::<T>,
        drop_fn: drop_impl::<T>,
        as_ref_fn: as_ref_impl::<T>,
        as_mut_fn: as_mut_impl::<T>,
    };
}

pub const STVAR_MAX_NESTING_DEPTH: i32 = 10;

pub struct STVar {
    storage: Storage,
    ops: &'static TypeOps,
    inline: bool,
}

unsafe impl Send for STVar {}
unsafe impl Sync for STVar {}

impl STVar {
    pub fn new<T>(value: T) -> Self
    where
        T: StBase + Clone + Send + Sync + 'static,
    {
        let ops = type_ops::<T>();

        if fits_inline::<T>() {
            let mut storage = Storage {
                inline: InlineStorage::uninit(),
            };
            unsafe {
                let ptr = inline_ptr_mut::<T>(&mut storage);
                ptr.write(value);
            }

            Self {
                storage,
                ops,
                inline: true,
            }
        } else {
            Self {
                storage: Storage {
                    heap: Box::into_raw(Box::new(value)).cast::<()>(),
                },
                ops,
                inline: false,
            }
        }
    }

    pub fn default_object(field: &'static SField) -> Self {
        Self::from_serialized_type(field.field_type(), field)
    }

    pub fn non_present_object(field: &'static SField) -> Self {
        Self::from_serialized_type(SerializedTypeId::NotPresent, field)
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField, depth: i32) -> Self {
        if depth > STVAR_MAX_NESTING_DEPTH {
            panic!("Maximum nesting depth of STVar exceeded");
        }

        match field.field_type() {
            SerializedTypeId::NotPresent => Self::new(STBaseValue::with_field(field)),
            SerializedTypeId::UInt8 => Self::new(STUInt8::from_serial_iter(sit, field)),
            SerializedTypeId::UInt16 => Self::new(STUInt16::from_serial_iter(sit, field)),
            SerializedTypeId::UInt32 => Self::new(STUInt32::from_serial_iter(sit, field)),
            SerializedTypeId::UInt64 => Self::new(STUInt64::from_serial_iter(sit, field)),
            SerializedTypeId::UInt128 => Self::new(STUInt128::from_serial_iter(sit, field)),
            SerializedTypeId::UInt160 => Self::new(STUInt160::from_serial_iter(sit, field)),
            SerializedTypeId::UInt192 => Self::new(STUInt192::from_serial_iter(sit, field)),
            SerializedTypeId::UInt256 => Self::new(STUInt256::from_serial_iter(sit, field)),
            SerializedTypeId::Amount => Self::new(STAmount::from_serial_iter(sit, field)),
            SerializedTypeId::Number => Self::new(STNumber::from_serial_iter(sit, field)),
            SerializedTypeId::Int32 => Self::new(STInt32::from_serial_iter(sit, field)),
            SerializedTypeId::Vector256 => Self::new(STVector256::from_serial_iter(sit, field)),
            SerializedTypeId::VariableLength => Self::new(STBlob::from_serial_iter(sit, field)),
            SerializedTypeId::Account => Self::new(STAccount::from_serial_iter(sit, field)),
            SerializedTypeId::Issue => Self::new(STIssue::from_serial_iter(sit, field)),
            SerializedTypeId::Object => Self::new(STObject::from_serial_iter(sit, field, depth)),
            SerializedTypeId::Array => Self::new(STArray::from_serial_iter(sit, field, depth)),
            SerializedTypeId::Currency => Self::new(STCurrency::from_serial_iter(sit, field)),
            SerializedTypeId::PathSet => Self::new(STPathSet::from_serial_iter(sit, field)),
            SerializedTypeId::XChainBridge => {
                Self::new(STXChainBridge::from_serial_iter(sit, field))
            }
            _ => panic!("Unknown object type"),
        }
    }

    pub fn from_serialized_type(type_id: SerializedTypeId, field: &'static SField) -> Self {
        assert!(
            matches!(type_id, SerializedTypeId::NotPresent) || type_id == field.field_type(),
            "xrpl::detail::STVar::STVar(SerializedTypeID) : valid type input"
        );

        match type_id {
            SerializedTypeId::NotPresent => Self::new(STBaseValue::with_field(field)),
            SerializedTypeId::UInt8 => Self::new(STUInt8::with_field(field, 0)),
            SerializedTypeId::UInt16 => Self::new(STUInt16::with_field(field, 0)),
            SerializedTypeId::UInt32 => Self::new(STUInt32::with_field(field, 0)),
            SerializedTypeId::UInt64 => Self::new(STUInt64::with_field(field, 0)),
            SerializedTypeId::UInt128 => {
                Self::new(STUInt128::with_field(field, Default::default()))
            }
            SerializedTypeId::UInt160 => {
                Self::new(STUInt160::with_field(field, Default::default()))
            }
            SerializedTypeId::UInt192 => {
                Self::new(STUInt192::with_field(field, Default::default()))
            }
            SerializedTypeId::UInt256 => {
                Self::new(STUInt256::with_field(field, Default::default()))
            }
            SerializedTypeId::Amount => Self::new(STAmount::with_field(field)),
            SerializedTypeId::Number => Self::new(STNumber::with_field(
                field,
                basics::number::NumberParts::zero(),
            )),
            SerializedTypeId::Int32 => Self::new(STInt32::with_field(field, 0)),
            SerializedTypeId::Vector256 => Self::new(STVector256::with_field(field)),
            SerializedTypeId::VariableLength => Self::new(STBlob::with_field(field)),
            SerializedTypeId::Account => Self::new(STAccount::with_field(field)),
            SerializedTypeId::Issue => Self::new(STIssue::with_field(field)),
            SerializedTypeId::Object => Self::new(STObject::new(field)),
            SerializedTypeId::Array => Self::new(STArray::new(field)),
            SerializedTypeId::Currency => Self::new(STCurrency::with_field(field)),
            SerializedTypeId::PathSet => Self::new(STPathSet::new(field)),
            SerializedTypeId::XChainBridge => Self::new(STXChainBridge::with_field(field)),
            _ => panic!("Unknown object type"),
        }
    }

    pub fn get(&self) -> &dyn StBase {
        unsafe { &*((self.ops.as_ref_fn)(self.raw_const_ptr())) }
    }

    pub fn get_mut(&mut self) -> &mut dyn StBase {
        unsafe { &mut *((self.ops.as_mut_fn)(self.raw_mut_ptr())) }
    }

    pub fn is_valid(&self) -> bool {
        self.get().is_valid()
    }

    pub fn check(&self) -> Result<(), ValidationError> {
        self.get().check()
    }

    fn raw_const_ptr(&self) -> *const () {
        unsafe {
            if self.inline {
                inline_ptr_const::<u8>(&self.storage).cast::<()>()
            } else {
                self.storage.heap.cast_const()
            }
        }
    }

    fn raw_mut_ptr(&mut self) -> *mut () {
        unsafe {
            if self.inline {
                inline_ptr_mut::<u8>(&mut self.storage).cast::<()>()
            } else {
                self.storage.heap
            }
        }
    }

    #[cfg(test)]
    fn stores_inline(&self) -> bool {
        self.inline
    }
}

impl Clone for STVar {
    fn clone(&self) -> Self {
        unsafe { (self.ops.clone_fn)(self.raw_const_ptr()) }
    }
}

impl Drop for STVar {
    fn drop(&mut self) {
        unsafe { (self.ops.drop_fn)(self.raw_mut_ptr(), self.inline) };
    }
}

impl PartialEq for STVar {
    fn eq(&self, other: &Self) -> bool {
        self.get().is_equivalent(other.get())
    }
}

impl Eq for STVar {}

impl std::fmt::Debug for STVar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("STVar")
            .field("stype", &self.get().stype())
            .field("field", &self.get().fname().name())
            .field("inline", &self.inline)
            .finish()
    }
}

impl std::ops::Deref for STVar {
    type Target = dyn StBase;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl std::ops::DerefMut for STVar {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

unsafe fn inline_ptr_const<T>(storage: &Storage) -> *const T {
    let inline = unsafe { ptr::addr_of!(storage.inline).cast::<InlineStorage>() };
    unsafe { (*inline).bytes.as_ptr().cast::<T>() }
}

unsafe fn inline_ptr_mut<T>(storage: &mut Storage) -> *mut T {
    let inline = unsafe { ptr::addr_of_mut!(storage.inline).cast::<InlineStorage>() };
    unsafe { (*inline).bytes.as_mut_ptr().cast::<T>() }
}

#[cfg(test)]
mod tests {
    use super::{STVAR_INLINE_SIZE, STVar, fits_inline};
    use crate::{STUInt32, StBase, StBaseCore, get_field_by_symbol};

    #[derive(Debug, Clone)]
    struct LargeTestValue {
        core: StBaseCore,
        payload: [u8; STVAR_INLINE_SIZE + 8],
    }

    impl LargeTestValue {
        fn new() -> Self {
            Self {
                core: StBaseCore::with_field(get_field_by_symbol("sfGeneric")),
                payload: [0xAB; STVAR_INLINE_SIZE + 8],
            }
        }
    }

    impl StBase for LargeTestValue {
        fn as_any(&self) -> &dyn std::any::Any {
            let _ = self.payload[0];
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }

        fn core(&self) -> &StBaseCore {
            &self.core
        }

        fn core_mut(&mut self) -> &mut StBaseCore {
            &mut self.core
        }
    }

    #[test]
    fn stvar_storage_uses_cpp_inline_threshold_for_small_values() {
        let value = STVar::new(STUInt32::with_field(get_field_by_symbol("sfSequence"), 7));
        let cloned = value.clone();

        assert_eq!(value, cloned);
        assert!(value.stores_inline());
        assert!(fits_inline::<STUInt32>());
    }

    #[test]
    fn stvar_storage_falls_back_to_heap_for_large_values() {
        let value = STVar::new(LargeTestValue::new());

        assert!(!value.stores_inline());
        assert!(!fits_inline::<LargeTestValue>());
    }

    #[test]
    fn stvar_inline_threshold_constant_contract() {
        assert_eq!(STVAR_INLINE_SIZE, 72);
        assert!(!fits_inline::<LargeTestValue>());
        assert!(fits_inline::<STUInt32>());
    }
}
