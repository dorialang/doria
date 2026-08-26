use std::collections::HashMap;

use crate::source::Span;
use crate::types::ResolvedType;

pub type EffectSiteMap = HashMap<Span, Vec<ResolvedType>>;
pub type CatchTypeMap = HashMap<Span, ResolvedType>;

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
