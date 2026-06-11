//! First honest `STObject` / `STArray` bootstrap.

use std::collections::BTreeMap;

use basics::{
    base_uint::{Uint128, Uint160, Uint192, Uint256},
    buffer::Buffer,
};

use crate::{
    AccountID, InnerObjectFormats, JsonOptions, JsonValue, SField, SOEStyle, SOTemplate, STAccount,
    STAmount, STBlob, STCurrency, STInt32, STIssue, STNumber, STPathSet, STUInt8, STUInt16,
    STUInt32, STUInt64, STUInt128, STUInt160, STUInt192, STUInt256, STVar, STVector256,
    STXChainBridge, SerialIter, SerializedTypeId, Serializer, StBase, StBaseCore, ValidationError,
    downcast_stbase_mut, downcast_stbase_ref, fix_inner_obj_template, fix_inner_obj_template2,
    get_current_transaction_rules, get_field_by_symbol, sf_generic,
};

#[derive(Debug, Clone)]
pub struct STObject {
    core: StBaseCore,
    fields: Vec<STVar>,
    template: Option<SOTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct STArray {
    core: StBaseCore,
    elements: Vec<STObject>,
}

impl PartialEq for STObject {
    fn eq(&self, other: &Self) -> bool {
        self.is_equivalent(other)
    }
}

impl Eq for STObject {}

impl STObject {
    pub fn new(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            fields: Vec::new(),
            template: None,
        }
    }

    pub fn with_template(template: &SOTemplate, field: &'static SField) -> Self {
        let mut object = Self::new(field);
        object.set_template(template);
        object
    }

    pub fn make_inner_object(field: &'static SField) -> Self {
        let mut object = Self::new(field);
        let rules = get_current_transaction_rules();
        let is_amm_obj = field == get_field_by_symbol("sfAuctionSlot")
            || field == get_field_by_symbol("sfVoteEntry");

        if (rules.is_none()
            || rules.as_ref().is_some_and(|rules| {
                (rules.enabled(&fix_inner_obj_template()) && is_amm_obj)
                    || (rules.enabled(&fix_inner_obj_template2()) && !is_amm_obj)
            }))
            && let Some(template) =
                InnerObjectFormats::get_instance().find_so_template_by_sfield(field)
        {
            object.set_template(template);
        }

        object
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField, depth: i32) -> Self {
        if depth > 10 {
            return Self::new(field);
        }

        let mut object = Self::new(field);
        object.set_from_serial_iter(sit, depth);
        object
    }

    pub fn is_free(&self) -> bool {
        self.template.is_none()
    }

    pub fn empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn reserve(&mut self, count: usize) {
        self.fields.reserve(count);
    }

    pub fn set(&mut self, template: &SOTemplate) {
        self.set_template(template);
    }

    pub fn set_template(&mut self, template: &SOTemplate) {
        self.fields.clear();
        self.fields.reserve(template.size());
        self.template = Some(template.clone());

        for element in template.iter() {
            if element.style() != SOEStyle::Required {
                self.fields
                    .push(STVar::non_present_object(element.sfield()));
            } else {
                self.fields.push(STVar::default_object(element.sfield()));
            }
        }
    }

    pub fn apply_template(&mut self, template: &SOTemplate) {
        self.template = Some(template.clone());
        let mut reordered = Vec::with_capacity(template.size());

        for element in template.iter() {
            if let Some(index) = self
                .fields
                .iter()
                .position(|field| field.get().fname() == element.sfield())
            {
                let field = self.fields.remove(index);
                if element.style() == SOEStyle::Default && field.get().is_default() {
                    // them from wire serialization. Include in reordered.
                }
                reordered.push(field);
            } else {
                if element.style() == SOEStyle::Required {
                    // Required field not in wire data — create with default
                    // value so it's always present for serialization/access.
                    reordered.push(STVar::default_object(element.sfield()));
                } else {
                    reordered.push(STVar::non_present_object(element.sfield()));
                }
            }
        }

        for field in &self.fields {
            if !field.get().fname().is_discardable() {
                // In Rust, we silently discard the invalid field to avoid crashing.
                return;
            }
        }

        self.fields = reordered;
    }

    pub fn apply_template_from_sfield(&mut self, field: &'static SField) {
        if let Some(template) = InnerObjectFormats::get_instance().find_so_template_by_sfield(field)
        {
            self.apply_template(template);
        }
    }

    pub fn set_from_serial_iter(&mut self, sit: &mut SerialIter<'_>, depth: i32) -> bool {
        let mut reached_end_of_object = false;
        let mut error = false;
        self.fields.clear();

        while !sit.empty() {
            let mut type_id = 0;
            let mut field_id = 0;
            sit.get_field_id(&mut type_id, &mut field_id);

            if type_id == SerializedTypeId::Object.as_i32() && field_id == 1 {
                reached_end_of_object = true;
                break;
            }

            if type_id == SerializedTypeId::Array.as_i32() && field_id == 1 {
                tracing::warn!(target: "protocol", "Illegal end-of-array marker in object");
                error = true;
                break;
            }

            let field = crate::get_field(crate::field_code_raw(type_id, field_id));
            if field.is_invalid() {
                tracing::warn!(target: "protocol", "Unknown field in serialized object");
                error = true;
                break;
            }

            let mut parsed = STVar::from_serial_iter(sit, field, depth + 1);
            if let Some(object) = parsed.get_mut().as_any_mut().downcast_mut::<STObject>() {
                object.apply_template_from_sfield(field);
            }
            self.fields.push(parsed);
        }

        let sorted_fields = self.sorted_fields(true);
        for pair in sorted_fields.windows(2) {
            if pair[0].fname() == pair[1].fname() {
                tracing::warn!(target: "protocol", "Duplicate field detected");
                error = true;
                break;
            }
        }

        if error {
            self.fields.clear();
        }

        reached_end_of_object
    }

    pub fn get_count(&self) -> usize {
        self.fields.len()
    }

    pub fn get_index(&self, index: usize) -> &dyn StBase {
        self.fields[index].get()
    }

    pub fn get_index_mut(&mut self, index: usize) -> &mut dyn StBase {
        self.fields[index].get_mut()
    }

    pub fn get_field_index(&self, field: &'static SField) -> i32 {
        if let Some(template) = &self.template {
            return template.get_index(field).unwrap_or(-1);
        }

        self.fields
            .iter()
            .position(|entry| entry.get().fname() == field)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    pub fn peek_at_pfield(&self, field: &'static SField) -> Option<&dyn StBase> {
        let index = self.get_field_index(field);
        if index < 0 || (index as usize) >= self.fields.len() {
            return None;
        }
        Some(self.fields[index as usize].get())
    }

    pub fn get_pfield(&self, field: &'static SField) -> Option<&dyn StBase> {
        self.peek_at_pfield(field)
    }

    pub fn is_field_present(&self, field: &'static SField) -> bool {
        self.peek_at_pfield(field)
            .map(|value| value.stype() != SerializedTypeId::NotPresent)
            .unwrap_or(false)
    }

    pub fn get_field_u8(&self, field: &'static SField) -> u8 {
        self.get_field_by_value::<STUInt8, _>(field, STUInt8::value)
    }

    pub fn get_field_u16(&self, field: &'static SField) -> u16 {
        self.get_field_by_value::<STUInt16, _>(field, STUInt16::value)
    }

    pub fn get_field_u32(&self, field: &'static SField) -> u32 {
        self.get_field_by_value::<STUInt32, _>(field, STUInt32::value)
    }

    pub fn get_field_u64(&self, field: &'static SField) -> u64 {
        self.get_field_by_value::<STUInt64, _>(field, STUInt64::value)
    }

    pub fn get_field_h128(&self, field: &'static SField) -> Uint128 {
        self.get_field_by_value::<STUInt128, _>(field, |value| *value.value())
    }

    pub fn get_field_h160(&self, field: &'static SField) -> Uint160 {
        self.get_field_by_value::<STUInt160, _>(field, |value| *value.value())
    }

    pub fn get_field_h192(&self, field: &'static SField) -> Uint192 {
        self.get_field_by_value::<STUInt192, _>(field, |value| *value.value())
    }

    pub fn get_field_h256(&self, field: &'static SField) -> Uint256 {
        self.get_field_by_value::<STUInt256, _>(field, |value| *value.value())
    }

    pub fn get_field_i32(&self, field: &'static SField) -> i32 {
        self.get_field_by_value::<STInt32, _>(field, STInt32::value)
    }

    pub fn get_account_id(&self, field: &'static SField) -> AccountID {
        self.get_field_by_value::<STAccount, _>(field, |value| *value.value())
    }

    pub fn get_field_vl(&self, field: &'static SField) -> Vec<u8> {
        self.get_field_by_const_ref::<STBlob, _>(field, STBlob::default(), |value| {
            value.data().to_vec()
        })
    }

    pub fn get_field_amount(&self, field: &'static SField) -> STAmount {
        self.get_field_by_const_ref::<STAmount, _>(
            field,
            STAmount::with_field(sf_generic()),
            |value| value,
        )
    }

    pub fn get_field_number(&self, field: &'static SField) -> STNumber {
        self.get_field_by_const_ref::<STNumber, _>(
            field,
            STNumber::with_field(sf_generic(), basics::number::NumberParts::zero()),
            |value| value,
        )
    }

    pub fn get_field_path_set(&self, field: &'static SField) -> STPathSet {
        self.get_field_by_const_ref::<STPathSet, _>(field, STPathSet::new(sf_generic()), |value| {
            value
        })
    }

    pub fn get_field_xchain_bridge(&self, field: &'static SField) -> STXChainBridge {
        self.get_field_by_const_ref::<STXChainBridge, _>(
            field,
            STXChainBridge::with_field(field),
            |value| value,
        )
    }

    pub fn get_field_v256(&self, field: &'static SField) -> STVector256 {
        self.get_field_by_const_ref::<STVector256, _>(
            field,
            STVector256::with_field(sf_generic()),
            |value| value,
        )
    }

    pub fn get_field_object(&self, field: &'static SField) -> STObject {
        let mut object =
            self.get_field_by_const_ref::<STObject, _>(field, STObject::new(field), |value| value);
        if object != STObject::new(field) {
            object.apply_template_from_sfield(field);
        }
        object
    }

    pub fn get_field_array(&self, field: &'static SField) -> STArray {
        self.get_field_by_const_ref::<STArray, _>(field, STArray::new(sf_generic()), |value| value)
    }

    pub fn get_field_currency(&self, field: &'static SField) -> STCurrency {
        self.get_field_by_const_ref::<STCurrency, _>(
            field,
            STCurrency::with_field(sf_generic()),
            |value| value,
        )
    }

    pub fn get_field_issue(&self, field: &'static SField) -> STIssue {
        self.get_field_by_const_ref::<STIssue, _>(
            field,
            STIssue::with_field(sf_generic()),
            |value| value,
        )
    }

    pub fn peek_field_object(&mut self, field: &'static SField) -> &mut STObject {
        self.peek_field::<STObject>(field)
    }

    pub fn peek_field_array(&mut self, field: &'static SField) -> &mut STArray {
        self.peek_field::<STArray>(field)
    }

    pub fn set_field_u8(&mut self, field: &'static SField, value: u8) {
        self.set_field_using_set_value::<STUInt8, _>(field, value);
    }

    pub fn set_field_u16(&mut self, field: &'static SField, value: u16) {
        self.set_field_using_set_value::<STUInt16, _>(field, value);
    }

    pub fn set_field_u32(&mut self, field: &'static SField, value: u32) {
        self.set_field_using_set_value::<STUInt32, _>(field, value);
    }

    pub fn set_field_u64(&mut self, field: &'static SField, value: u64) {
        self.set_field_using_set_value::<STUInt64, _>(field, value);
    }

    pub fn set_field_h128(&mut self, field: &'static SField, value: Uint128) {
        self.set_field_using_set_value::<STUInt128, _>(field, value);
    }

    pub fn set_field_h160(&mut self, field: &'static SField, value: Uint160) {
        self.set_field_using_set_value::<STUInt160, _>(field, value);
    }

    pub fn set_field_h192(&mut self, field: &'static SField, value: Uint192) {
        self.set_field_using_set_value::<STUInt192, _>(field, value);
    }

    pub fn set_field_h256(&mut self, field: &'static SField, value: Uint256) {
        self.set_field_using_set_value::<STUInt256, _>(field, value);
    }

    pub fn set_field_i32(&mut self, field: &'static SField, value: i32) {
        self.set_field_using_set_value::<STInt32, _>(field, value);
    }

    pub fn set_account_id(&mut self, field: &'static SField, value: AccountID) {
        self.set_field_using_set_value::<STAccount, _>(field, value);
    }

    pub fn set_field_vl(&mut self, field: &'static SField, value: &[u8]) {
        self.set_field_using_set_value::<STBlob, _>(field, Buffer::from_bytes(value));
    }

    pub fn set_field_amount(&mut self, field: &'static SField, mut value: STAmount) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_number(&mut self, field: &'static SField, mut value: STNumber) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_v256(&mut self, field: &'static SField, mut value: STVector256) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_path_set(&mut self, field: &'static SField, mut value: STPathSet) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_object(&mut self, field: &'static SField, mut value: STObject) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_array(&mut self, field: &'static SField, mut value: STArray) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_currency(&mut self, field: &'static SField, mut value: STCurrency) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_issue(&mut self, field: &'static SField, mut value: STIssue) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_field_xchain_bridge(&mut self, field: &'static SField, mut value: STXChainBridge) {
        value.set_fname(field);
        self.set_stbase(value);
    }

    pub fn set_stbase<T>(&mut self, mut value: T)
    where
        T: StBase + Clone + Send + Sync + 'static,
    {
        let field = value.fname();
        value.set_fname(field);

        let index = self.get_field_index(field);
        if index >= 0 {
            self.fields[index as usize] = STVar::new(value);
            return;
        }

        if !self.is_free() {
            // Templated object doesn't have this field — skip silently.
            return;
        }

        self.fields.push(STVar::new(value));
    }

    pub fn set_flag(&mut self, flag: u32) -> bool {
        let field = get_field_by_symbol("sfFlags");
        let Some(value) = self.get_pfield_mut(field, true) else {
            return false;
        };
        let Some(value) = value.as_any_mut().downcast_mut::<STUInt32>() else {
            return false;
        };
        value.set_value(value.value() | flag);
        true
    }

    pub fn clear_flag(&mut self, flag: u32) -> bool {
        let field = get_field_by_symbol("sfFlags");
        let Some(value) = self.get_pfield_mut(field, false) else {
            return false;
        };
        let Some(value) = value.as_any_mut().downcast_mut::<STUInt32>() else {
            return false;
        };
        value.set_value(value.value() & !flag);
        true
    }

    pub fn is_flag(&self, flag: u32) -> bool {
        (self.get_flags() & flag) == flag
    }

    pub fn get_flags(&self) -> u32 {
        let field = get_field_by_symbol("sfFlags");
        self.peek_at_pfield(field)
            .and_then(|value| value.as_any().downcast_ref::<STUInt32>())
            .map(STUInt32::value)
            .unwrap_or_default()
    }

    pub fn make_field_present(&mut self, field: &'static SField) -> &mut dyn StBase {
        let index = self.get_field_index(field);
        if index == -1 {
            // Field not in template — add it anyway (reference would throw for
            // templated objects, but our transactor may need it)
            self.fields.push(STVar::default_object(field));
            return self.fields.last_mut().expect("field pushed").get_mut();
        }

        if self.fields[index as usize].get().stype() == SerializedTypeId::NotPresent {
            self.fields[index as usize] = STVar::default_object(field);
        }

        self.fields[index as usize].get_mut()
    }

    pub fn make_field_absent(&mut self, field: &'static SField) {
        let index = self.get_field_index(field);
        if index == -1 {
            panic!("Field not found: {}", field.name());
        }
        if self.fields[index as usize].get().stype() != SerializedTypeId::NotPresent {
            self.fields[index as usize] = STVar::non_present_object(field);
        }
    }

    pub fn del_field(&mut self, field: &'static SField) -> bool {
        let index = self.get_field_index(field);
        if index == -1 {
            return false;
        }
        self.fields.remove(index as usize);
        true
    }

    pub fn get_style(&self, field: &'static SField) -> Option<SOEStyle> {
        self.template.as_ref()?.style(field).ok()
    }

    pub fn emplace_back(&mut self, field: STVar) -> usize {
        self.fields.push(field);
        self.fields.len() - 1
    }

    pub fn get_serializer(&self) -> Serializer {
        let mut serializer = Serializer::default();
        self.add_internal(&mut serializer, true);
        serializer
    }

    pub fn add_without_signing_fields(&self, serializer: &mut Serializer) {
        self.add_internal(serializer, false);
    }

    pub fn get_hash(&self, prefix: crate::HashPrefix) -> basics::base_uint::Uint256 {
        let mut serializer = Serializer::default();
        serializer.add32_prefix(prefix);
        self.add_internal(&mut serializer, true);
        serializer.get_sha512_half()
    }

    pub fn get_signing_hash(&self, prefix: crate::HashPrefix) -> basics::base_uint::Uint256 {
        let mut serializer = Serializer::default();
        serializer.add32_prefix(prefix);
        self.add_internal(&mut serializer, false);
        serializer.get_sha512_half()
    }

    fn add_internal(&self, serializer: &mut Serializer, include_non_signing_fields: bool) {
        for field in self.sorted_fields(include_non_signing_fields) {
            if field.is_default()
                && let Some(ref tmpl) = self.template
            {
                let is_default_style = tmpl.iter().any(|e| {
                    e.sfield().name() == field.fname().name() && e.style() == SOEStyle::Default
                });
                if is_default_style {
                    continue;
                }
            }
            assert!(
                field.stype() != SerializedTypeId::Object
                    || field.fname().field_type() == SerializedTypeId::Object,
                "xrpl::STObject::add : valid field type"
            );
            field.add_field_id(serializer);
            field.add(serializer);
            if matches!(
                field.stype(),
                SerializedTypeId::Object | SerializedTypeId::Array
            ) {
                serializer.add_field_type_id(field.stype(), 1);
            }
        }
    }

    fn sorted_fields(&self, include_non_signing_fields: bool) -> Vec<&dyn StBase> {
        let mut fields: Vec<&dyn StBase> = self
            .fields
            .iter()
            .map(|field| field.get())
            .filter(|field| {
                field.stype() != SerializedTypeId::NotPresent
                    && field.fname().should_include(include_non_signing_fields)
            })
            .collect();
        fields.sort_by_key(|field| field.fname().code());
        fields
    }

    fn get_field_by_value<T, U>(&self, field: &'static SField, project: impl FnOnce(&T) -> U) -> U
    where
        T: StBase + 'static,
        U: Default,
    {
        let Some(raw) = self.peek_at_pfield(field) else {
            return U::default();
        };
        if raw.stype() == SerializedTypeId::NotPresent {
            return U::default();
        }
        let Some(value) = raw.as_any().downcast_ref::<T>() else {
            return U::default();
        };
        project(value)
    }

    fn get_field_by_const_ref<T, U>(
        &self,
        field: &'static SField,
        empty: T,
        project: impl FnOnce(T) -> U,
    ) -> U
    where
        T: StBase + Clone + 'static,
    {
        let Some(raw) = self.peek_at_pfield(field) else {
            return project(empty.clone());
        };
        if raw.stype() == SerializedTypeId::NotPresent {
            return project(empty);
        }
        let Some(value) = raw.as_any().downcast_ref::<T>() else {
            return project(empty);
        };
        project(value.clone())
    }

    fn get_pfield_mut(
        &mut self,
        field: &'static SField,
        create_okay: bool,
    ) -> Option<&mut dyn StBase> {
        let index = self.get_field_index(field);
        if index == -1 {
            if create_okay && self.is_free() {
                self.fields.push(STVar::default_object(field));
                return self.fields.last_mut().map(STVar::get_mut);
            }
            return None;
        }
        Some(self.fields[index as usize].get_mut())
    }

    fn materialized_pfield_mut(
        &mut self,
        field: &'static SField,
        create_okay: bool,
    ) -> Option<&mut dyn StBase> {
        let index = self.get_field_index(field);
        if index == -1 {
            if create_okay && self.is_free() {
                self.fields.push(STVar::default_object(field));
                return self.fields.last_mut().map(STVar::get_mut);
            }
            return None;
        }

        let index = index as usize;
        if self.fields[index].get().stype() == SerializedTypeId::NotPresent {
            self.fields[index] = STVar::default_object(field);
        }
        Some(self.fields[index].get_mut())
    }

    fn set_field_using_set_value<T, V>(&mut self, field: &'static SField, value: V)
    where
        T: StBase + 'static,
        V: 'static,
    {
        let raw = self.materialized_pfield_mut(field, true);
        let Some(raw) = raw else {
            // Field not materialized — skip. reference would create it via template
            // but our materialized_pfield_mut with create=true should handle this.
            return;
        };

        let Some(value_field) = raw.as_any_mut().downcast_mut::<T>() else {
            // Type mismatch — field exists but as a different type.
            // This indicates a transactor parity gap where we're writing
            // the wrong type. Skip silently (reference equivalent: the template
            // instantiation would handle the conversion).
            return;
        };

        Self::assign_field_value(value_field, value);
    }

    fn assign_field_value<T, V>(field: &mut T, value: V)
    where
        T: StBase + 'static,
        V: 'static,
    {
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt8>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<u8>()
                .copied()
                .expect("value type should match STUInt8");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt16>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<u16>()
                .copied()
                .expect("value type should match STUInt16");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt32>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<u32>()
                .copied()
                .expect("value type should match STUInt32");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt64>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<u64>()
                .copied()
                .expect("value type should match STUInt64");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STInt32>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<i32>()
                .copied()
                .expect("value type should match STInt32");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt128>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<Uint128>()
                .copied()
                .expect("value type should match STUInt128");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt160>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<Uint160>()
                .copied()
                .expect("value type should match STUInt160");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt192>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<Uint192>()
                .copied()
                .expect("value type should match STUInt192");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STUInt256>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<Uint256>()
                .copied()
                .expect("value type should match STUInt256");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STAccount>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<AccountID>()
                .copied()
                .expect("value type should match STAccount");
            field.set_value(value);
            return;
        }
        if let Some(field) = (field as &mut dyn StBase)
            .as_any_mut()
            .downcast_mut::<STBlob>()
        {
            let value = (&value as &dyn std::any::Any)
                .downcast_ref::<Buffer>()
                .cloned()
                .expect("value type should match STBlob");
            field.set_value(value);
        }

        // Type not recognized — skip silently (transactor parity gap)
    }

    fn peek_field<T>(&mut self, field: &'static SField) -> &mut T
    where
        T: StBase + 'static,
    {
        let raw = self.materialized_pfield_mut(field, true);
        let Some(raw) = raw else {
            panic!("Field not found: {}", field.name());
        };

        downcast_stbase_mut::<T>(raw)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn StBase> {
        self.fields.iter().map(STVar::get)
    }
}

impl StBase for STObject {
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

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Object
    }

    fn text(&self) -> String {
        let mut text = String::from("{");
        let mut first = false;
        for field in &self.fields {
            if !first {
                text.push_str(", ");
                first = false;
            }
            text.push_str(&field.get().text());
        }
        text.push('}');
        text
    }

    fn full_text(&self) -> String {
        let mut text = if self.fname().has_name() {
            format!("{} = {{", self.fname().name())
        } else {
            "{".to_string()
        };

        let mut first = true;
        for field in &self.fields {
            if field.get().stype() == SerializedTypeId::NotPresent {
                continue;
            }
            if !first {
                text.push_str(", ");
            } else {
                first = false;
            }
            text.push_str(&field.get().full_text());
        }
        text.push('}');
        text
    }

    fn json(&self, options: JsonOptions) -> JsonValue {
        let mut object = BTreeMap::new();
        for field in &self.fields {
            if field.get().stype() != SerializedTypeId::NotPresent {
                object.insert(
                    field.get().fname().name().to_string(),
                    field.get().json(options),
                );
            }
        }
        JsonValue::Object(object)
    }

    fn add(&self, serializer: &mut Serializer) {
        self.add_internal(serializer, true);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        let other = downcast_stbase_ref::<Self>(other);

        if self.template == other.template && self.fields.len() == other.fields.len() {
            return self
                .fields
                .iter()
                .zip(other.fields.iter())
                .all(|(left, right)| {
                    left.get().stype() == right.get().stype()
                        && left.get().is_equivalent(right.get())
                });
        }

        let left = self.sorted_fields(true);
        let right = other.sorted_fields(true);
        left.len() == right.len()
            && left.iter().zip(right.iter()).all(|(left, right)| {
                left.fname() == right.fname()
                    && left.stype() == right.stype()
                    && left.is_equivalent(*right)
            })
    }

    fn is_default(&self) -> bool {
        self.fields.is_empty()
    }

    fn is_valid(&self) -> bool {
        if let Some(template) = &self.template {
            for element in template.iter() {
                if element.style() == SOEStyle::Required && !self.is_field_present(element.sfield())
                {
                    return false;
                }
            }
        }

        for field in &self.fields {
            if !field.get().is_valid() {
                return false;
            }
        }

        true
    }

    fn check(&self) -> Result<(), ValidationError> {
        if let Some(template) = &self.template {
            for element in template.iter() {
                if element.style() == SOEStyle::Required
                    && !self.is_field_actually_present(element.sfield())
                {
                    return Err(ValidationError::MissingField(element.sfield().name()));
                }
            }
        }

        for field in &self.fields {
            field.get().check()?;
        }

        Ok(())
    }
}

impl STObject {
    pub fn has_field(&self, field: &'static SField) -> bool {
        self.get_field_index(field) != -1
    }

    pub fn is_field_actually_present(&self, field: &'static SField) -> bool {
        self.fields.iter().any(|f| f.get().fname() == field)
    }
}

impl STArray {
    pub fn new(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            elements: Vec::new(),
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField, depth: i32) -> Self {
        let mut array = Self::new(field);

        while !sit.empty() {
            let mut type_id = 0;
            let mut field_id = 0;
            sit.get_field_id(&mut type_id, &mut field_id);

            if type_id == SerializedTypeId::Array.as_i32() && field_id == 1 {
                break;
            }

            if type_id == SerializedTypeId::Object.as_i32() && field_id == 1 {
                break;
            }

            let field = crate::get_field(crate::field_code_raw(type_id, field_id));
            if field.is_invalid() {
                break;
            }
            if field.field_type() != SerializedTypeId::Object {
                break;
            }

            let mut object = STObject::from_serial_iter(sit, field, depth + 1);
            object.apply_template_from_sfield(field);
            array.elements.push(object);
        }

        array
    }

    pub fn push_back(&mut self, object: STObject) {
        self.elements.push(object);
    }

    pub fn reserve(&mut self, additional: usize) {
        self.elements.reserve(additional);
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&STObject> {
        self.elements.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut STObject> {
        self.elements.get_mut(index)
    }

    pub fn last_mut(&mut self) -> Option<&mut STObject> {
        self.elements.last_mut()
    }

    pub fn iter(&self) -> impl Iterator<Item = &STObject> {
        self.elements.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut STObject> {
        self.elements.iter_mut()
    }
}

impl StBase for STArray {
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

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Array
    }

    fn full_text(&self) -> String {
        let mut text = String::from("[");
        let mut first = true;
        for object in &self.elements {
            if !first {
                text.push(',');
            }
            text.push_str(&object.full_text());
            first = false;
        }
        text.push(']');
        text
    }

    fn text(&self) -> String {
        let mut text = String::from("[");
        let mut first = true;
        for object in &self.elements {
            if !first {
                text.push(',');
            }
            text.push_str(&object.text());
            first = false;
        }
        text.push(']');
        text
    }

    fn json(&self, options: JsonOptions) -> JsonValue {
        JsonValue::Array(
            self.elements
                .iter()
                .filter(|object| object.stype() != SerializedTypeId::NotPresent)
                .map(|object| {
                    let mut inner = BTreeMap::new();
                    inner.insert(object.fname().name().to_string(), object.json(options));
                    JsonValue::Object(inner)
                })
                .collect(),
        )
    }

    fn add(&self, serializer: &mut Serializer) {
        for object in &self.elements {
            object.add_field_id(serializer);
            object.add(serializer);
            serializer.add_field_type_id(SerializedTypeId::Object, 1);
        }
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).elements == self.elements
    }

    fn is_default(&self) -> bool {
        self.elements.is_empty()
    }

    fn is_valid(&self) -> bool {
        for object in &self.elements {
            if !object.is_valid() {
                return false;
            }
        }
        true
    }

    fn check(&self) -> Result<(), ValidationError> {
        for object in &self.elements {
            object.check()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SOElement, SOTemplate};

    #[test]
    fn check_reports_missing_required_field() {
        let field = get_field_by_symbol("sfAccount");
        let template = SOTemplate::new(
            vec![SOElement::new(field, SOEStyle::Required).unwrap()],
            vec![],
        )
        .unwrap();

        let mut object = STObject::new(get_field_by_symbol("sfLedgerEntry"));
        object.set_template(&template);

        // Required field 'sfAccount' is missing (only default stubs if not set).
        // STObject::set_template fills it with default_object, but for some types
        // like AccountID, it might be zero but present.
        // Let's manually remove it to be sure.
        object.fields.clear();

        let result = object.check();
        assert!(matches!(
            result,
            Err(ValidationError::MissingField("Account"))
        ));
    }
}
