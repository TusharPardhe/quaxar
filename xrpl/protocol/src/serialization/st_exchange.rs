//! `STExchange.h`-style typed field access helpers over `STObject`.

use basics::{
    base_uint::{Uint128, Uint160, Uint192, Uint256},
    buffer::Buffer,
};

use crate::{
    AccountID, SField, STAmount, STArray, STCurrency, STIssue, STNumber, STObject, STPathSet,
    STVector256, STXChainBridge,
};

pub trait StExchangeValue: Sized {
    fn get(st: &STObject, field: &'static SField) -> Option<Self>;
    fn set(st: &mut STObject, field: &'static SField, value: Self);
}

macro_rules! impl_copy_exchange_value {
    ($ty:ty, $getter:ident, $setter:ident) => {
        impl StExchangeValue for $ty {
            fn get(st: &STObject, field: &'static SField) -> Option<Self> {
                st.is_field_present(field).then(|| st.$getter(field))
            }

            fn set(st: &mut STObject, field: &'static SField, value: Self) {
                st.$setter(field, value);
            }
        }
    };
}

macro_rules! impl_clone_exchange_value {
    ($ty:ty, $getter:ident, $setter:ident) => {
        impl StExchangeValue for $ty {
            fn get(st: &STObject, field: &'static SField) -> Option<Self> {
                st.is_field_present(field).then(|| st.$getter(field))
            }

            fn set(st: &mut STObject, field: &'static SField, value: Self) {
                st.$setter(field, value);
            }
        }
    };
}

impl_copy_exchange_value!(u8, get_field_u8, set_field_u8);
impl_copy_exchange_value!(u16, get_field_u16, set_field_u16);
impl_copy_exchange_value!(u32, get_field_u32, set_field_u32);
impl_copy_exchange_value!(u64, get_field_u64, set_field_u64);
impl_copy_exchange_value!(i32, get_field_i32, set_field_i32);
impl_copy_exchange_value!(Uint128, get_field_h128, set_field_h128);
impl_copy_exchange_value!(Uint160, get_field_h160, set_field_h160);
impl_copy_exchange_value!(Uint192, get_field_h192, set_field_h192);
impl_copy_exchange_value!(Uint256, get_field_h256, set_field_h256);
impl_copy_exchange_value!(AccountID, get_account_id, set_account_id);

impl StExchangeValue for Vec<u8> {
    fn get(st: &STObject, field: &'static SField) -> Option<Self> {
        st.is_field_present(field).then(|| st.get_field_vl(field))
    }

    fn set(st: &mut STObject, field: &'static SField, value: Self) {
        st.set_field_vl(field, &value);
    }
}

impl StExchangeValue for Buffer {
    fn get(st: &STObject, field: &'static SField) -> Option<Self> {
        st.is_field_present(field)
            .then(|| Buffer::from_bytes(&st.get_field_vl(field)))
    }

    fn set(st: &mut STObject, field: &'static SField, value: Self) {
        st.set_field_vl(field, value.data());
    }
}

impl_clone_exchange_value!(STAmount, get_field_amount, set_field_amount);
impl_clone_exchange_value!(STNumber, get_field_number, set_field_number);
impl_clone_exchange_value!(STPathSet, get_field_path_set, set_field_path_set);
impl_clone_exchange_value!(STVector256, get_field_v256, set_field_v256);
impl_clone_exchange_value!(STObject, get_field_object, set_field_object);
impl_clone_exchange_value!(STArray, get_field_array, set_field_array);
impl_clone_exchange_value!(STCurrency, get_field_currency, set_field_currency);
impl_clone_exchange_value!(STIssue, get_field_issue, set_field_issue);
impl_clone_exchange_value!(
    STXChainBridge,
    get_field_xchain_bridge,
    set_field_xchain_bridge
);

pub fn get<T: StExchangeValue>(st: &STObject, field: &'static SField) -> Option<T> {
    T::get(st, field)
}

pub fn set<T: StExchangeValue>(st: &mut STObject, field: &'static SField, value: T) {
    T::set(st, field, value);
}

pub fn set_blob_with(
    st: &mut STObject,
    field: &'static SField,
    size: usize,
    init: impl FnOnce(&mut [u8]),
) {
    let mut buffer = Buffer::with_size(size);
    init(buffer.data_mut());
    st.set_field_vl(field, buffer.data());
}

pub fn set_blob_bytes(st: &mut STObject, field: &'static SField, data: &[u8]) {
    st.set_field_vl(field, data);
}

pub fn erase(st: &mut STObject, field: &'static SField) {
    st.make_field_absent(field);
}
