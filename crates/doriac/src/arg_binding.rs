//! Name-resolution binding for call arguments (decision 0098).
//!
//! Binding maps each call-site argument onto a parameter *before* type
//! inference. Positional arguments bind by position; named arguments (`name:
//! value`) bind to the parameter of that name and may appear in any order and
//! skip parameters that have defaults. The parser guarantees that positional
//! arguments never follow named ones, so positional arguments are always a
//! contiguous prefix.
//!
//! This is a pure function of parameter names/defaults and the argument names.
//! Every pass that needs the argument→parameter mapping — semantic checking,
//! ownership analysis, and MIR lowering — computes it from the same helper so
//! the mapping is identical everywhere. Semantic checking is the authoritative
//! pass for the duplicate/unknown/missing diagnostics; because ownership and MIR
//! lowering only run once semantic analysis reports no errors, they always see a
//! valid binding.

/// The result of binding call arguments to parameters.
#[derive(Debug, Clone, Default)]
pub struct BoundArguments {
    /// For each parameter (in declaration order), the source-argument index
    /// bound to it, or `None` when the parameter was omitted and its default
    /// applies.
    pub param_to_arg: Vec<Option<usize>>,
    /// For each source argument (in written order), the parameter index it
    /// binds to, or `None` when the argument could not be bound (an unknown
    /// name, or a positional argument beyond the last parameter).
    pub arg_to_param: Vec<Option<usize>>,
    /// Source-argument indices whose name matched no parameter.
    pub unknown: Vec<usize>,
    /// Source-argument indices that bind a parameter already bound by an earlier
    /// argument (supplied positionally and by name, or named twice).
    pub duplicate: Vec<usize>,
    /// Parameter indices with no argument and no default (missing required).
    pub missing: Vec<usize>,
    /// Number of positional arguments beyond the last parameter (arity
    /// overflow); the caller reports this as "too many arguments".
    pub overflow: usize,
}

impl BoundArguments {
    /// Whether any named argument appears in the call. Positional-only calls can
    /// keep their existing positional code paths unchanged.
    pub fn has_named(arg_names: &[Option<&str>]) -> bool {
        arg_names.iter().any(Option::is_some)
    }
}

/// Bind `arg_names` (one entry per source argument, `Some(name)` for a named
/// argument) to the parameters described by `param_names`/`param_has_default`.
pub fn bind_arguments(
    param_names: &[&str],
    param_has_default: &[bool],
    arg_names: &[Option<&str>],
) -> BoundArguments {
    debug_assert_eq!(param_names.len(), param_has_default.len());

    let mut param_to_arg = vec![None; param_names.len()];
    let mut arg_to_param = vec![None; arg_names.len()];
    let mut unknown = Vec::new();
    let mut duplicate = Vec::new();
    let mut overflow = 0usize;
    let mut next_positional = 0usize;

    for (arg_index, name) in arg_names.iter().enumerate() {
        let param_index = match name {
            None => {
                let index = next_positional;
                next_positional += 1;
                if index >= param_names.len() {
                    overflow += 1;
                    continue;
                }
                index
            }
            Some(name) => match param_names.iter().position(|candidate| candidate == name) {
                Some(index) => index,
                None => {
                    unknown.push(arg_index);
                    continue;
                }
            },
        };

        if param_to_arg[param_index].is_some() {
            duplicate.push(arg_index);
            continue;
        }
        param_to_arg[param_index] = Some(arg_index);
        arg_to_param[arg_index] = Some(param_index);
    }

    let missing = param_to_arg
        .iter()
        .enumerate()
        .filter(|(index, bound)| bound.is_none() && !param_has_default[*index])
        .map(|(index, _)| index)
        .collect();

    BoundArguments {
        param_to_arg,
        arg_to_param,
        unknown,
        duplicate,
        missing,
        overflow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_only_binds_by_position() {
        let bound = bind_arguments(&["a", "b"], &[false, false], &[None, None]);
        assert_eq!(bound.param_to_arg, vec![Some(0), Some(1)]);
        assert_eq!(bound.arg_to_param, vec![Some(0), Some(1)]);
        assert!(bound.unknown.is_empty());
        assert!(bound.duplicate.is_empty());
        assert!(bound.missing.is_empty());
        assert_eq!(bound.overflow, 0);
    }

    #[test]
    fn named_reorders_by_name() {
        // f(b: _, a: _)
        let bound = bind_arguments(&["a", "b"], &[false, false], &[Some("b"), Some("a")]);
        assert_eq!(bound.param_to_arg, vec![Some(1), Some(0)]);
        assert_eq!(bound.arg_to_param, vec![Some(1), Some(0)]);
    }

    #[test]
    fn named_skips_defaulted_middle() {
        // f(a: _, c: _) with (a, b = default, c)
        let bound = bind_arguments(
            &["a", "b", "c"],
            &[false, true, false],
            &[Some("a"), Some("c")],
        );
        assert_eq!(bound.param_to_arg, vec![Some(0), None, Some(1)]);
        assert!(bound.missing.is_empty());
    }

    #[test]
    fn skipping_required_middle_is_missing() {
        // f(a: _, c: _) with (a, b required, c)
        let bound = bind_arguments(
            &["a", "b", "c"],
            &[false, false, false],
            &[Some("a"), Some("c")],
        );
        assert_eq!(bound.missing, vec![1]);
    }

    #[test]
    fn positional_and_named_same_param_is_duplicate() {
        // f(_, a: _) with (a, b)
        let bound = bind_arguments(&["a", "b"], &[false, false], &[None, Some("a")]);
        assert_eq!(bound.duplicate, vec![1]);
    }

    #[test]
    fn unknown_name_is_reported() {
        let bound = bind_arguments(&["a"], &[false], &[Some("z")]);
        assert_eq!(bound.unknown, vec![0]);
        assert_eq!(bound.missing, vec![0]);
    }

    #[test]
    fn positional_overflow_counts() {
        let bound = bind_arguments(&["a"], &[false], &[None, None, None]);
        assert_eq!(bound.overflow, 2);
    }
}
