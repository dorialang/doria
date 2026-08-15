use std::collections::HashMap;

use crate::source::Span;
use crate::types::ResolvedType;

pub type EffectSiteMap = HashMap<(usize, usize), Vec<ResolvedType>>;
pub type CatchTypeMap = HashMap<(usize, usize), ResolvedType>;

pub(crate) fn record_effect_site(
    sites: &mut EffectSiteMap,
    span: Span,
    effects: impl IntoIterator<Item = ResolvedType>,
) {
    let site = sites.entry((span.start, span.end)).or_default();
    for effect in effects {
        if !site.contains(&effect) {
            site.push(effect);
        }
    }
}

pub(crate) fn effects_at(sites: &EffectSiteMap, span: Span) -> &[ResolvedType] {
    sites
        .get(&(span.start, span.end))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(crate) fn effect_is_caught(effect: &ResolvedType, catch: &ResolvedType) -> bool {
    catch == &ResolvedType::Error || catch == effect
}
