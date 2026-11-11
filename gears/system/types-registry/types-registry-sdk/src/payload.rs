//! Store-independent payload compilation from a projected registry snapshot.

use gts::{PayloadValidator, ResolvedType};
use serde_json::Value;

use crate::{EntityField, GtsTypeId, TypeSchema};

/// Why a projected Type Schema could not produce a payload validator.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CompilePayloadValidatorError {
    /// Read the schema again with this field selected; compilation performs no I/O.
    #[error("type schema {type_id} requires projection field {field:?}")]
    MissingField {
        type_id: GtsTypeId,
        field: EntityField,
    },
    /// The selected authored document is not an object (including JSON null).
    #[error("authored content of type schema {type_id} must be an object")]
    InvalidContent { type_id: GtsTypeId },
    /// GTS refused an abstract type or could not compile its resolved schema.
    #[error("GTS payload validator compilation failed: {0}")]
    Upstream(#[from] gts::CompilePayloadValidatorError),
}

impl TypeSchema {
    /// Compile a reusable, store-independent GTS payload validator.
    ///
    /// Select [`EntityField::Content`] and [`EntityField::ResolvedSchema`]. Abstractness
    /// comes from the authored leaf's `x-gts-abstract`, never an inherited value in
    /// the resolved schema. Other authored-schema admission checks are not repeated.
    /// Traits, origin and provenance are not required.
    ///
    /// The returned [`PayloadValidator`] preserves the native compiled GTS rules,
    /// including dialect, format, regex and GTS reference constraints. Reference
    /// syntax/patterns are checked without looking up targets (`GtsRefValidation::None`).
    /// The resolved schema must contain every resource it references; lossy reference
    /// inlining can produce verdicts different from store-based validation.
    ///
    /// Compile once and reuse for many payloads. No registry reads, artifact resolution,
    /// caching, defaults insertion or lifecycle/availability checks occur here. Consumers
    /// choose the snapshot freshness and lifecycle policy and supply their own envelopes.
    ///
    /// # Errors
    /// [`CompilePayloadValidatorError::MissingField`] for an unselected required field,
    /// [`CompilePayloadValidatorError::InvalidContent`] for non-object authored content,
    /// or [`CompilePayloadValidatorError::Upstream`] retaining the native compilation error.
    /// A selected null resolved schema reaches the GTS compiler and is not a missing field.
    pub fn compile_payload_validator(
        &self,
    ) -> Result<PayloadValidator, CompilePayloadValidatorError> {
        let required = |field| CompilePayloadValidatorError::MissingField {
            type_id: self.type_id.clone(),
            field,
        };
        let content = self
            .content
            .as_ref()
            .ok_or_else(|| required(EntityField::Content))?;
        let schema = self
            .resolved_schema
            .as_ref()
            .ok_or_else(|| required(EntityField::ResolvedSchema))?;
        if !content.is_object() {
            return Err(CompilePayloadValidatorError::InvalidContent {
                type_id: self.type_id.clone(),
            });
        }

        // gts 0.16.x's payload compiler reads only id, is_abstract and schema.
        // Neutral values for its unused fields avoid requiring extra projections;
        // this temporary ResolvedType is never exposed as a resolved registry snapshot.
        let resolved = ResolvedType {
            id: self.type_id.clone(),
            is_abstract: content.get("x-gts-abstract") == Some(&Value::Bool(true)),
            is_final: false,
            schema: schema.clone(),
            effective_traits: Value::Null,
            effective_traits_schema: Value::Null,
        };
        resolved.compile_payload_validator().map_err(Into::into)
    }
}
