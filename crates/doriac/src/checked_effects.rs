use std::collections::HashMap;

use crate::source::Span;
use crate::types::ResolvedType;

pub type EffectSiteMap = HashMap<Span, Vec<ResolvedType>>;
pub type CatchTypeMap = HashMap<Span, ResolvedType>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedEffectClass {
    Required,
    AmbientIo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckedEffectProfile {
    pub required: Vec<ResolvedType>,
    pub ambient: Vec<ResolvedType>,
}

impl CheckedEffectProfile {
    pub fn classify(effects: impl IntoIterator<Item = ResolvedType>) -> Self {
        let mut profile = Self::default();
        for effect in effects {
            let target = match classify_effect(&effect) {
                CheckedEffectClass::Required => &mut profile.required,
                CheckedEffectClass::AmbientIo => &mut profile.ambient,
            };
            if !target.contains(&effect) {
                target.push(effect);
            }
        }
        profile
    }
}

pub fn classify_effect(effect: &ResolvedType) -> CheckedEffectClass {
    match effect {
        ResolvedType::Class(class)
            if matches!(
                class.name.as_str(),
                crate::compiler_known_io::IO_ERROR | crate::compiler_known_io::INVALID_UTF8_ERROR
            ) =>
        {
            CheckedEffectClass::AmbientIo
        }
        _ => CheckedEffectClass::Required,
    }
}

pub fn is_ambient_io_effect(effect: &ResolvedType) -> bool {
    classify_effect(effect) == CheckedEffectClass::AmbientIo
}

pub(crate) fn record_effect_site(
    sites: &mut EffectSiteMap,
    span: Span,
    effects: impl IntoIterator<Item = ResolvedType>,
) {
    let site = sites.entry(span).or_default();
    for effect in effects {
        if !site.contains(&effect) {
            site.push(effect);
        }
    }
}

pub(crate) fn effects_at(sites: &EffectSiteMap, span: Span) -> &[ResolvedType] {
    sites.get(&span).map(Vec::as_slice).unwrap_or_default()
}

pub(crate) fn effect_is_caught(effect: &ResolvedType, catch: &ResolvedType) -> bool {
    catch == &ResolvedType::Error || catch == effect
}
