//! Protocol `SOElement` / `SOTemplate` registry layer.

use crate::sfield::SField;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SOEStyle {
    Invalid = -1,
    Required = 0,
    Optional = 1,
    Default = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SOETxMPTIssue {
    None,
    Supported,
    NotSupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SOElement {
    sfield: &'static SField,
    style: SOEStyle,
    support_mpt: SOETxMPTIssue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    NonUsefulField {
        field_code: i32,
        field_name: &'static str,
    },
    InvalidFieldIndex,
    DuplicateFieldIndex,
}

impl SOElement {
    pub fn new(sfield: &'static SField, style: SOEStyle) -> Result<Self, TemplateError> {
        Self::new_with_mpt(sfield, style, SOETxMPTIssue::None)
    }

    pub fn new_with_mpt(
        sfield: &'static SField,
        style: SOEStyle,
        support_mpt: SOETxMPTIssue,
    ) -> Result<Self, TemplateError> {
        if !sfield.is_useful() {
            return Err(TemplateError::NonUsefulField {
                field_code: sfield.code(),
                field_name: sfield.name(),
            });
        }
        Ok(Self {
            sfield,
            style,
            support_mpt,
        })
    }

    pub const fn sfield(self) -> &'static SField {
        self.sfield
    }

    pub const fn style(self) -> SOEStyle {
        self.style
    }

    pub const fn support_mpt(self) -> SOETxMPTIssue {
        self.support_mpt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SOTemplate {
    elements: Vec<SOElement>,
    indices: Vec<i32>,
}

impl SOTemplate {
    pub fn new(
        mut unique_fields: Vec<SOElement>,
        common_fields: Vec<SOElement>,
    ) -> Result<Self, TemplateError> {
        let mut indices = vec![-1; crate::sfield::max_sfield_num() + 1];

        unique_fields.extend(common_fields);

        for (index, element) in unique_fields.iter().enumerate() {
            let field_num = element.sfield.field_num();
            if field_num == 0 || field_num >= indices.len() {
                return Err(TemplateError::InvalidFieldIndex);
            }
            if indices[field_num] != -1 {
                return Err(TemplateError::DuplicateFieldIndex);
            }
            indices[field_num] = index as i32;
        }

        Ok(Self {
            elements: unique_fields,
            indices,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &SOElement> {
        self.elements.iter()
    }

    pub fn elements(&self) -> &[SOElement] {
        &self.elements
    }

    pub fn size(&self) -> usize {
        self.elements.len()
    }

    pub fn get_index(&self, sfield: &'static SField) -> Result<i32, TemplateError> {
        let field_num = sfield.field_num();
        if field_num == 0 || field_num >= self.indices.len() {
            return Err(TemplateError::InvalidFieldIndex);
        }
        Ok(self.indices[field_num])
    }

    pub fn style(&self, sfield: &'static SField) -> Result<SOEStyle, TemplateError> {
        let index = self.get_index(sfield)?;
        if index < 0 {
            return Err(TemplateError::InvalidFieldIndex);
        }
        Ok(self.elements[index as usize].style())
    }
}
