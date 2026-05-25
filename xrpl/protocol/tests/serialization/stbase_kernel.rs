use protocol::{
    JsonOptions, JsonValue, SField, SerializedTypeId, Serializer, StBase, StBaseCore,
    downcast_stbase_mut, downcast_stbase_ref, get_field_by_symbol, st_base_eq, st_base_ne, to_json,
};

struct NotPresentValue {
    core: StBaseCore,
}

impl NotPresentValue {
    fn new() -> Self {
        Self {
            core: StBaseCore::new(),
        }
    }
}

impl StBase for NotPresentValue {
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

struct TextValue {
    core: StBaseCore,
    text: &'static str,
    equivalent_key: u32,
}

impl TextValue {
    fn new(field: &'static SField, text: &'static str, equivalent_key: u32) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            text,
            equivalent_key,
        }
    }
}

impl StBase for TextValue {
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
        SerializedTypeId::UInt32
    }

    fn text(&self) -> String {
        self.text.to_string()
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<TextValue>(other).equivalent_key == self.equivalent_key
    }
}

#[test]
fn json_options_match_cpp_flag_mask_rules() {
    assert_eq!(JsonOptions::NONE.bits(), 0);
    assert_eq!(JsonOptions::ALL.bits(), 0b11);
    assert_eq!(
        (JsonOptions::INCLUDE_DATE | JsonOptions::DISABLE_API_PRIOR_V2).bits(),
        JsonOptions::ALL.bits()
    );
    assert_eq!((JsonOptions::ALL & JsonOptions::INCLUDE_DATE).bits(), 0b01);
    assert_eq!((!JsonOptions::NONE).bits(), JsonOptions::ALL.bits());
    assert_eq!((!JsonOptions::ALL).bits(), JsonOptions::NONE.bits());
}

#[test]
fn stbase_defaults_match_cpp_base_behavior() {
    let value = NotPresentValue::new();

    assert_eq!(value.fname(), get_field_by_symbol("sfGeneric"));
    assert_eq!(value.stype(), SerializedTypeId::NotPresent);
    assert_eq!(value.text(), "");
    assert_eq!(
        value.json(JsonOptions::NONE),
        JsonValue::String(String::new())
    );
    assert_eq!(value.full_text(), "");
    assert!(value.is_default());
    assert_eq!(to_json(&value), JsonValue::String(String::new()));
}

#[test]
fn stbase_field_id_and_assignment_quirk_match_cpp() {
    let account = get_field_by_symbol("sfAccount");
    let sequence = get_field_by_symbol("sfSequence");

    let mut actual = Serializer::default();
    TextValue::new(sequence, "7", 1).add_field_id(&mut actual);

    let mut expected = Serializer::default();
    expected.add_field_id(sequence.field_type().as_i32(), sequence.field_value());
    assert_eq!(actual, expected);

    let mut destination = StBaseCore::with_field(get_field_by_symbol("sfInvalid"));
    destination.assign_from(&StBaseCore::with_field(account));
    assert_eq!(destination.fname(), account);

    let mut useful_destination = StBaseCore::with_field(sequence);
    useful_destination.assign_from(&StBaseCore::with_field(account));
    assert_eq!(useful_destination.fname(), sequence);
}

#[test]
fn stbase_equality_rules() {
    let left = TextValue::new(get_field_by_symbol("sfSequence"), "7", 10);
    let equal = TextValue::new(get_field_by_symbol("sfSequence"), "7", 10);
    let non_equivalent = TextValue::new(get_field_by_symbol("sfSequence"), "7", 20);
    let not_present = NotPresentValue::new();

    assert!(st_base_eq(&left, &equal));
    assert!(!st_base_ne(&left, &equal));
    assert!(!st_base_eq(&left, &non_equivalent));
    assert!(st_base_ne(&left, &non_equivalent));
    assert!(!st_base_eq(&left, &not_present));
    assert!(st_base_ne(&left, &not_present));
    assert!(not_present.is_equivalent(&NotPresentValue::new()));
}

#[test]
fn stbase_full_text_and_downcast_match_cpp_shape() {
    let named = TextValue::new(get_field_by_symbol("sfSequence"), "7", 1);
    assert_eq!(named.full_text(), "Sequence = 7");

    let unnamed = TextValue::new(get_field_by_symbol("sfGeneric"), "payload", 1);
    assert_eq!(unnamed.full_text(), "payload");

    let mut value: Box<dyn StBase> =
        Box::new(TextValue::new(get_field_by_symbol("sfSequence"), "7", 99));
    assert_eq!(downcast_stbase_ref::<TextValue>(&*value).equivalent_key, 99);
    downcast_stbase_mut::<TextValue>(&mut *value).equivalent_key = 100;
    assert_eq!(
        downcast_stbase_ref::<TextValue>(&*value).equivalent_key,
        100
    );
}

#[test]
#[should_panic(expected = "xrpl::STBase::add should never be called")]
fn stbase_add_panics_by_default() {
    NotPresentValue::new().add(&mut Serializer::default());
}

#[test]
#[should_panic(expected = "bad cast")]
fn stbase_downcast_panics_on_wrong_type() {
    let value: Box<dyn StBase> = Box::new(NotPresentValue::new());
    let _ = downcast_stbase_ref::<TextValue>(&*value);
}
