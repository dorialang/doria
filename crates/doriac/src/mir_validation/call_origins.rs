//! Derive storage origins through executable calls and closure environments.
//!
//! A nominal preparation entry lends the complete registered receiver and the
//! group's byte region. Helpers and closure environments receive those actual
//! origins. Descriptor cleanup does not retire the request. The group dispatch
//! and retirement boundary must validate the eventual handoff separately.

use super::*;
use crate::class_layout::PropertyId;
use retained::possible_callees;

#[path = "writeback.rs"]
mod writeback;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Pending,
    Writeback,
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;

    #[test]
    fn copies_of_local_values_share_what_neither_changed() {
        let mut values = LocalValues::new(3);
        values[1] = Value::owner(Root::Group);
        let mut copy = values.clone();
        copy[2].unknown_callable = true;
        assert!(std::rc::Rc::ptr_eq(&values.0[0], &copy.0[0]));
        assert!(std::rc::Rc::ptr_eq(&values.0[1], &copy.0[1]));
        assert!(!std::rc::Rc::ptr_eq(&values.0[2], &copy.0[2]));

        // The copy already holds everything the original does.
        assert!(!copy.join(&values, false));
        assert!(!copy.join(&values, true));
        assert!(std::rc::Rc::ptr_eq(&values.0[1], &copy.0[1]));

        // The original gains the one value the copy changed and nothing else.
        assert!(values.join(&copy, false));
        assert!(values == copy);
        assert!(std::rc::Rc::ptr_eq(&values.0[0], &copy.0[0]));
    }

    #[test]
    fn a_back_edge_still_widens_values_two_copies_share() {
        let root = Owner::new(Root::Local(0, Some(mir::LocalId(0))));
        let nested = root
            .projected(Projection::Element(None))
            .projected(Projection::Element(None));
        let mut values = LocalValues::new(1);
        values[0].owners = HashSet::from([root.clone(), nested.clone()]);
        let shared = values.clone();

        // An ordinary join of a value with itself changes nothing.
        assert!(!values.join(&shared, false));
        // Across a back edge the longer owner widens against the shorter one,
        // exactly as it does when the two copies do not share the value.
        let mut expected = Value {
            owners: HashSet::from([root, nested]),
            ..Value::default()
        };
        let incoming = expected.clone();
        let changed = expected.join_on_cycle(&incoming, true);
        assert_eq!(values.join(&shared, true), changed);
        assert!(values[0] == expected);
    }

    #[test]
    fn copy_enum_construction_keeps_region_origins_in_plain_and_nullable_fields() {
        let program = crate::lower_source_to_mir(
            "enum-region-origins.doria",
            "use Doria\\Std\\Io\\ByteRegion;\nenum Packet { case View(ByteRegion $view); }\nfunction main(): void {}",
        )
        .unwrap();
        let (mut proof, _) = solve(&program, false);
        // Validation checks every payload enum type against its declaration,
        // so the fixture uses the program's own definition and layout.
        let definition = program
            .enums
            .iter()
            .find(|definition| definition.name == "Packet")
            .unwrap();
        let ty = definition.payload_type().unwrap();
        assert!(ty.capabilities.copy, "a region payload enum is Copy");
        let case = crate::enums::EnumCaseId {
            enum_id: definition.id,
            index: 0,
        };
        let region = mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Local {
            kind: crate::types::ByteRegionKind::Readonly,
            local: mir::LocalId(0),
            transfer: false,
        });
        let payload = mir::PayloadEnumExpression::Construct {
            ty,
            case,
            fields: vec![region],
            span: Span::default(),
        };
        let site = Site {
            block: mir::BlockId(0),
            statement: 0,
            call: 0,
        };
        for value in [
            mir::Rvalue::PayloadEnum(payload.clone()),
            mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(payload)),
        ] {
            for (source, can_retain) in [
                (Root::Group, true),
                (Root::Local(0, Some(mir::LocalId(0))), false),
            ] {
                let mut flow = Flow {
                    values: vec![Value::owner(source)].into(),
                    fields: HashMap::new(),
                    selected: HashSet::new(),
                    writes: HashMap::new(),
                };
                let mut result = proof.aggregate_value(0, site, &mut flow, &value).unwrap();
                let owner = Owner::new(Root::Temporary(0, site));
                assert!(result.owners.contains(&owner));
                let projection = Projection::Payload(case, 0);
                let field = flow.field(&owner.projected(projection.clone())).unwrap();
                assert_eq!(field.owners, HashSet::from([Owner::new(source)]));
                let destination = Owner::new(Root::Local(0, Some(mir::LocalId(1))));
                flow.rehome(&mut result, destination.clone());
                let field = flow.field(&destination.projected(projection)).unwrap();
                assert_eq!(field.owners, HashSet::from([Owner::new(source)]));
                let selection = Selection {
                    request: Root::Request(program.selected_entry.unwrap(), mir::LocalId(2)),
                    storage: field.owners.into_iter().next().unwrap(),
                    writable: false,
                };
                assert_eq!(selection.validate().is_ok(), can_retain);
            }
        }
    }

    fn solve(program: &mir::Program, exhaustive: bool) -> (Proof<'_>, usize) {
        let function = program.selected_entry.unwrap();
        assert!(program.functions[function.0].params.is_empty());
        let mut proof = Proof::new(program, Purpose::Writeback).unwrap();
        proof
            .contexts
            .push(Context::new(function, None, Kind::Root, OWN, 0, Vec::new()));
        let mut visits = 0;
        loop {
            while proof.changed {
                proof.changed = false;
                let live = proof.live();
                for context in 0..proof.contexts.len() {
                    if !live.get(context).copied().unwrap_or(true) {
                        continue;
                    }
                    proof.contexts[context].dirty |= exhaustive;
                    visits += usize::from(proof.contexts[context].dirty);
                    proof.body(context, false).unwrap();
                }
            }
            if !proof.seed_callable_definitions().unwrap() {
                break;
            }
        }
        (proof, visits)
    }

    #[test]
    fn normal_return_proof_preserves_unknown_call_contexts() {
        let program = crate::lower_source_to_mir(
            "normal-return-proof.doria",
            r#"
class Failure implements Error { string $message = "failure"; }
function succeed(): int throws Failure { return 7; }
function fail(): int throws Failure { throw new Failure(); }
function main(): void {
    try { echo succeed(); echo fail(); } catch (Failure) {}
}
"#,
        )
        .unwrap();
        let (proof, _) = solve(&program, false);
        for (name, expected) in [("succeed", true), ("fail", false)] {
            let (caller, site, callee) = proof
                .calls
                .keys()
                .find(|(_, _, callee, _)| program.functions[callee.0].name == name)
                .map(|(caller, site, callee, _)| (*caller, *site, *callee))
                .unwrap();
            assert_eq!(
                proof
                    .direct_call_may_return(caller, site, callee, &[])
                    .unwrap(),
                expected,
            );
            let unestablished = Site {
                statement: usize::MAX,
                ..site
            };
            assert!(proof
                .direct_call_may_return(caller, unestablished, callee, &[])
                .unwrap());
        }
    }

    #[test]
    fn dependency_scheduling_matches_exhaustive_proofs_and_final_checks() {
        let mut saved_visits = 0;
        for source in [
            r#"
function leaf(int $value): int { return $value; }
function middle(int $value): int { return leaf($value); }
function main(): int { return middle(7); }
"#,
            r#"
function repeat(function(writable ?int): void $run, writable ?int $value, int $depth): void {
    if ($depth > 0) { repeat($run, $value, $depth - 1); }
    else { $run($value); }
}
function main(): void {
    writable ?int $value = 7;
    let $run = function(writable ?int $value): void { $value = ($value ?? 0) + 1; };
    repeat($run, $value, 2);
    echo ($value ?? 0);
}
"#,
            r#"
function defer(writable ?int $value): function writable(): void {
    return function(): void with (writable $value) { $value = ($value ?? 0) + 1; };
}
function main(): void {
    writable ?int $value = 7;
    let writable $pending = defer($value);
}
"#,
        ] {
            let program = crate::lower_source_to_mir("proof-scheduling.doria", source)
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
            let (mut selective, visits) = solve(&program, false);
            let (exhaustive, all_visits) = solve(&program, true);
            assert_eq!(selective.contexts, exhaustive.contexts, "{source}");
            assert_eq!(selective.calls, exhaustive.calls, "{source}");
            assert_eq!(selective.environments, exhaustive.environments, "{source}");
            assert_eq!(selective.definitions, exhaustive.definitions, "{source}");
            assert!(visits <= all_visits);
            saved_visits += all_visits - visits;
            selective.check().unwrap();
            assert!(
                !selective.changed,
                "final checks must not discover new facts"
            );
            assert_eq!(selective.contexts, exhaustive.contexts, "{source}");
            assert_eq!(selective.calls, exhaustive.calls, "{source}");
            assert_eq!(selective.environments, exhaustive.environments, "{source}");
            assert_eq!(selective.definitions, exhaustive.definitions, "{source}");
        }
        assert!(saved_visits > 0);
    }

    #[test]
    fn final_checking_never_skips_a_clean_context() {
        let program =
            crate::lower_source_to_mir("clean-proof-context.doria", "function main(): void {}")
                .unwrap();
        let (mut proof, _) = solve(&program, false);
        assert!(!proof.contexts[0].dirty);
        proof.contexts[0].function = mir::FunctionId(program.functions.len());
        proof.body(0, false).unwrap();
        assert!(proof.body(0, true).is_err());
    }

    #[test]
    fn changed_environments_refresh_executable_and_existing_definition_bindings() {
        let program = crate::lower_source_to_mir(
            "changed-proof-environment.doria",
            r#"
class Counter { writable int $value = 7; }
function defer(writable Counter $owner): function writable(): void {
    return function(): void with (writable $owner) { $owner->value++; };
}
function main(): void {
    let writable $owner = new Counter();
    let writable $pending = defer($owner);
}
"#,
        )
        .unwrap();
        let (mut proof, _) = solve(&program, false);
        // Abandoned smaller-entry summaries of `defer` keep unseeded definitions.
        let key = *proof
            .definitions
            .iter()
            .find(|(_, definition)| definition.context.is_some())
            .unwrap()
            .0;
        let (creator, descriptor) = key;
        let definition = proof.definitions[&key].context.unwrap();
        let closure = proof.definitions[&key].closure.clone();
        let function = program.closure_descriptors[descriptor.0].entry_function;
        let site = Site {
            block: program.functions[function.0].entry_block,
            statement: 0,
            call: 0,
        };
        let flow = Flow {
            values: LocalValues::new(0),
            fields: HashMap::new(),
            selected: HashSet::new(),
            writes: HashMap::new(),
        };
        // The creator executes the callback across a component boundary, and
        // the callback re-entering itself folds into the definition proof.
        let Call::Record(record) = proof
            .context(creator, site, function, Some(closure.clone()), &[], &flow)
            .unwrap()
        else {
            panic!("a component-crossing callback call is summarized");
        };
        assert_eq!(
            proof
                .context(
                    definition,
                    site,
                    function,
                    Some(closure.clone()),
                    &[],
                    &flow
                )
                .unwrap(),
            Call::Context(definition)
        );
        for context in &mut proof.contexts {
            context.dirty = false;
        }
        proof.changed = false;
        proof.environments.get_mut(&key).unwrap()[0].unknown_contents = true;
        proof.refresh_environment(creator, descriptor).unwrap();
        assert!(proof.changed);
        for reader in [creator, definition] {
            assert!(proof.contexts[reader].dirty);
        }
        assert!(proof.contexts[definition]
            .captures
            .values()
            .any(|value| value.unknown_contents));
        // Revisiting the executable call re-binds it to the changed environment.
        proof
            .context(creator, site, function, Some(closure), &[], &flow)
            .unwrap();
        let summary = proof.records[record].summary.unwrap();
        assert!(proof.contexts[summary]
            .captures
            .values()
            .any(|value| value.unknown_contents));
        let field = proof.environments[&key][0]
            .owners
            .iter()
            .next()
            .unwrap()
            .project(PropertyId {
                class: ClassId(0),
                index: 0,
            });
        for context in &mut proof.contexts {
            context.dirty = false;
        }
        proof.changed = false;
        proof
            .definitions
            .get_mut(&key)
            .unwrap()
            .fields
            .entry(field.clone())
            .or_default()
            .unknown_contents = true;
        proof.refresh_environment(creator, descriptor).unwrap();
        assert!(proof.contexts[definition].dirty);
        assert!(proof.contexts[definition].fields[&field].unknown_contents);
        assert!(proof.changed);
    }
}

#[cfg(test)]
mod join_tests {
    use super::*;

    #[test]
    fn value_join_reports_each_new_fact_but_not_duplicates() {
        let owner = Owner::new(Root::Local(0, Some(mir::LocalId(0))));
        let facts = [
            Value::owner(owner.root),
            Value {
                closures: HashSet::from([Closure {
                    creator: 0,
                    descriptor: mir::ClosureDescriptorId(0),
                    home: owner,
                }]),
                ..Value::default()
            },
            Value {
                homes: HashSet::from([None]),
                ..Value::default()
            },
            Value {
                types: HashSet::from([None]),
                ..Value::default()
            },
            Value {
                boxed_homes: HashSet::from([None]),
                ..Value::default()
            },
            Value {
                unknown_callable: true,
                ..Value::default()
            },
            Value {
                unknown_contents: true,
                ..Value::default()
            },
        ];
        let mut value = Value::default();
        assert!(!value.join(&Value::default()));
        for fact in facts {
            assert!(value.join(&fact));
            assert!(!value.join(&fact));
        }
        let before = value.clone();
        assert!(!value.join(&before));
        assert_eq!(value, before);
    }

    #[test]
    fn storage_joins_report_new_slots_even_before_their_value_is_known() {
        let owner = Owner::new(Root::Local(0, Some(mir::LocalId(0))));
        let incoming = HashMap::from([(owner.clone(), Value::default())]);
        for join in [join_fields, join_writes] {
            let mut fields = HashMap::new();
            assert!(join(&mut fields, &incoming));
            assert!(!join(&mut fields, &incoming));
            assert_eq!(fields, incoming);
            let known = HashMap::from([(owner.clone(), Value::owner(owner.root))]);
            assert!(join(&mut fields, &known));
            assert!(!join(&mut fields, &known));
            assert_eq!(fields, known);
        }
    }

    #[test]
    fn recursive_storage_summaries_preserve_distinct_member_facts() {
        let child = PropertyId {
            class: ClassId(0),
            index: 0,
        };
        let callback = PropertyId {
            class: ClassId(0),
            index: 1,
        };
        let root = Owner::new(Root::Local(0, Some(mir::LocalId(0))));
        let temporary = Owner::new(Root::Local(0, Some(mir::LocalId(1))));
        let home = root.project(callback);
        let stored = Value {
            owners: HashSet::from([home.clone()]),
            closures: HashSet::from([Closure {
                creator: 0,
                descriptor: mir::ClosureDescriptorId(0),
                home: home.clone(),
            }]),
            homes: HashSet::from([None]),
            types: HashSet::from([None]),
            unknown_callable: true,
            ..Value::default()
        };
        let mut flow = Flow {
            values: vec![Value::owner(root.root)].into(),
            fields: HashMap::from([(home, stored.clone())]),
            selected: HashSet::new(),
            writes: HashMap::new(),
        };
        let wrap = |flow: &mut Flow| {
            let mut incoming = flow.clone();
            let mut previous = incoming.values[0].clone();
            let selected = temporary.project(child);
            incoming.rehome(&mut previous, selected.clone());
            incoming.fields.entry(selected).or_default().join(&previous);
            let mut constructed = Value::owner(temporary.root);
            incoming.rehome(&mut constructed, root.clone());
            incoming.values[0] = constructed;
            incoming.writes.clone_from(&incoming.fields);
            flow.join(incoming, true)
        };
        for _ in 0..4 {
            wrap(&mut flow);
        }
        for _ in 0..12 {
            assert!(
                !wrap(&mut flow),
                "recursive storage facts must reach a fixed point"
            );
        }
        let mut descendant = root;
        for _ in 0..8 {
            descendant = descendant.project(child);
        }
        let object = flow.field(&descendant).expect("summarized child storage");
        assert!(!object.unknown_callable);
        assert!(object.closures.is_empty());
        let callable = flow
            .field(&descendant.project(callback))
            .expect("summarized callback storage");
        assert!(callable.unknown_callable);
        assert_eq!(callable.homes, stored.homes);
        assert_eq!(callable.types, stored.types);
        assert!(!callable.closures.is_empty());
        let summarized = callable
            .owners
            .iter()
            .find(|owner| owner.opaque)
            .unwrap()
            .clone();
        let child_summary = object.owners.iter().find(|owner| owner.opaque).unwrap();
        assert!(!flow.field(child_summary).unwrap().unknown_callable);
        flow.values.push(Value {
            owners: HashSet::from([descendant.project(callback)]),
            ..Value::default()
        });
        flow.values.push(Value {
            owners: HashSet::from([descendant]),
            ..Value::default()
        });
        flow.write(
            &Value {
                owners: HashSet::from([summarized.clone()]),
                ..Value::default()
            },
            &Value {
                unknown_contents: true,
                ..Value::default()
            },
        );
        assert!(flow.values[1].unknown_contents);
        assert!(!flow.values[2].unknown_contents);
        let mut mixed = Value {
            owners: HashSet::from([summarized.clone()]),
            ..Value::default()
        }
        .mixed_payload();
        let projected = mixed.owners.drain().next().unwrap();
        assert_eq!(projected.fields, summarized.fields);
        assert_eq!(projected.member, Some(Projection::MixedPayload));
    }

    #[test]
    fn summarized_collection_members_preserve_unknown_index_overlap() {
        let root = Owner::new(Root::Local(0, Some(mir::LocalId(0))));
        let mut all = root.projected(Projection::Element(None));
        all.widen_on_cycle(std::iter::once(&root));
        let mut indexed = root.projected(Projection::Element(Some(
            crate::ownership::cleanup::CollectionKey::Integer(7),
        )));
        indexed.widen_on_cycle(std::iter::once(&root));
        assert!(all.covers(&indexed));
        assert!(!indexed.covers(&all));
        assert!(indexed.storage_overlaps(&all));
        assert!(!all.storage_overlaps(&root.projected(Projection::Key)));
    }
}

#[cfg(test)]
mod construction_tests {
    #[test]
    fn repeatedly_wrapped_owned_objects_preserve_proof_convergence() {
        let program = crate::lower_source_to_mir(
            "wrapped-owned-origins.doria",
            r#"
class Node { function __construct(take ?Node $child) {} }
function main(): void {
    let writable $node = new Node(null);
    foreach (0..<3 as int $index) { $node = new Node($node); }
    echo "wrapped";
}
"#,
        )
        .unwrap();
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.exit_status, 0);
        assert_eq!(output.stdout, b"wrapped");
    }

    #[test]
    fn repeated_aggregate_construction_preserves_distinct_object_origins() {
        let program = crate::lower_source_to_mir(
            "aggregate-construction-origins.doria",
            r#"
class Item { int $value = 42; }
class Holder { function __construct(take Item $item) {} }
function main(): void {
    let writable $iteration = 0;
    while ($iteration < 3) {
        List<Item> $plain = [new Item(), new Item()];
        List<Holder> $nested = [new Holder(new Item()), new Holder(new Item())];
        echo $plain[0]->value;
        echo $plain[1]->value;
        echo $nested[0]->item->value;
        echo $nested[1]->item->value;
        $iteration++;
    }
}
"#,
        )
        .unwrap();
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.exit_status, 0);
        assert_eq!(output.stdout, b"424242424242424242424242");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Root {
    Adapter,
    Group,
    Request(mir::FunctionId, mir::LocalId),
    Local(usize, Option<mir::LocalId>),
    Temporary(usize, Site),
    Construction(usize, Site, Span),
    Environment(usize, mir::ClosureDescriptorId),
    Static(mir::StaticId),
    Unknown,
    // Caller storage named by a summary's entry; see `Scope`.
    Foreign(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Projection {
    Property(PropertyId),
    Capture(mir::ClosureEnvironmentFieldId),
    Element(Option<crate::ownership::cleanup::CollectionKey>),
    Key,
    Payload(crate::enums::EnumCaseId, usize),
    MixedPayload,
}

impl Projection {
    fn covers(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Element(left), Self::Element(right)) => left.is_none() || left == right,
            _ => self == other,
        }
    }
}

fn literal_collection_key(value: &mir::Rvalue) -> Option<crate::ownership::cleanup::CollectionKey> {
    use crate::ownership::cleanup::CollectionKey as K;
    match value {
        mir::Rvalue::Value(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Integer(value)),
            ..
        })) => Some(K::Integer(value.mathematical_value())),
        mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(value)),
        })) => Some(K::Bool(*value)),
        mir::Rvalue::Value(mir::ValueExpression::Enum(mir::EnumExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Enum(value)),
            ..
        })) => Some(K::Enum(*value)),
        mir::Rvalue::String(mir::StringExpression::Literal(value)) => {
            Some(K::String(value.clone()))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Owner {
    root: Root,
    fields: Vec<Projection>,
    opaque: bool,
    // Recursive descendants still distinguish the final typed storage member.
    member: Option<Projection>,
}

impl Owner {
    fn new(root: Root) -> Self {
        Self {
            root,
            fields: Vec::new(),
            opaque: false,
            member: None,
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.root == other.root
            && self
                .fields
                .iter()
                .zip(&other.fields)
                .all(|(left, right)| match (left, right) {
                    (Projection::Element(left), Projection::Element(right)) => {
                        left.is_none() || right.is_none() || left == right
                    }
                    _ => left == right,
                })
    }

    fn project(&self, property: PropertyId) -> Self {
        self.projected(Projection::Property(property))
    }

    fn projected(&self, projection: Projection) -> Self {
        let mut owner = self.clone();
        if !owner.opaque {
            owner.fields.push(projection);
        } else {
            owner.member = Some(projection);
        }
        owner
    }

    fn capture(&self, field: mir::ClosureEnvironmentFieldId) -> Self {
        self.projected(Projection::Capture(field))
    }

    fn covers(&self, other: &Self) -> bool {
        self.opaque
            && self.root == other.root
            && self.member.as_ref().is_none_or(|member| {
                if other.opaque {
                    other.member.as_ref()
                } else {
                    other.fields.last()
                }
                .is_some_and(|other| member.covers(other))
            })
            && self.fields.len() <= other.fields.len()
            && self
                .fields
                .iter()
                .zip(&other.fields)
                .all(|(left, right)| left.covers(right))
    }

    fn storage_overlaps(&self, other: &Self) -> bool {
        !self.opaque
            && !other.opaque
            && self.fields.len() == other.fields.len()
            && self.overlaps(other)
            || self.covers(other)
            || other.covers(self)
    }

    fn summarized<'a>(&self, previous: impl Iterator<Item = &'a Self>) -> Self {
        previous
            .filter(|owner| owner.covers(self))
            .min_by_key(|owner| owner.fields.len())
            .cloned()
            .unwrap_or_else(|| self.clone())
    }

    fn relocate(&mut self, source: &Self, destination: &Self) {
        if self.root == source.root && self.fields.starts_with(&source.fields) {
            let member = if self.fields.len() == source.fields.len()
                && (!self.opaque || source.opaque && self.member == source.member)
            {
                destination
                    .member
                    .clone()
                    .or_else(|| destination.fields.last().cloned())
            } else {
                self.member.clone().or_else(|| self.fields.last().cloned())
            };
            self.root = destination.root;
            if destination.opaque {
                self.fields.clone_from(&destination.fields);
            } else {
                self.fields
                    .splice(..source.fields.len(), destination.fields.clone());
            }
            self.opaque |= source.opaque || destination.opaque;
            if self.opaque {
                self.member = member;
            }
        }
    }

    fn widen_on_cycle<'a>(&mut self, previous: impl Iterator<Item = &'a Self>) {
        let member = self.member.clone().or_else(|| self.fields.last().cloned());
        let prefix = previous
            .filter(|owner| self.root == owner.root && self.fields.len() > owner.fields.len())
            .filter_map(|owner| {
                if self.fields.starts_with(&owner.fields) {
                    Some(owner.fields.len())
                } else if !owner.opaque
                    && member.as_ref() == owner.fields.last()
                    && self.fields[..self.fields.len() - 1]
                        .starts_with(&owner.fields[..owner.fields.len() - 1])
                {
                    Some(owner.fields.len() - 1)
                } else {
                    None
                }
            })
            .min();
        if let Some(prefix) = prefix {
            self.fields.truncate(prefix);
            self.opaque = true;
            self.member = member;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Closure {
    creator: usize,
    descriptor: mir::ClosureDescriptorId,
    home: Owner,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Value {
    owners: HashSet<Owner>,
    closures: HashSet<Closure>,
    homes: HashSet<Option<mir::Type>>,
    types: HashSet<Option<mir::Type>>,
    boxed_homes: HashSet<Option<mir::Type>>,
    unknown_callable: bool,
    unknown_contents: bool,
}

impl Value {
    fn owner(root: Root) -> Self {
        Self {
            owners: HashSet::from([Owner::new(root)]),
            closures: HashSet::new(),
            homes: HashSet::new(),
            types: HashSet::new(),
            boxed_homes: HashSet::new(),
            unknown_callable: false,
            unknown_contents: false,
        }
    }

    fn mark_unknown_callable(&mut self, ty: mir::Type) {
        self.unknown_contents |= ty.has_move_ownership();
        self.unknown_callable |= matches!(
            ty,
            mir::Type::Function(_)
                | mir::Type::NullableFunction(_)
                | mir::Type::Mixed
                | mir::Type::NullableMixed
        );
    }

    fn mixed_payload(mut self) -> Self {
        self.homes.clone_from(&self.boxed_homes);
        if self.homes.is_empty() {
            self.homes.insert(None);
        }
        self.owners = self
            .owners
            .into_iter()
            .map(|owner| owner.projected(Projection::MixedPayload))
            .collect();
        self
    }

    fn join(&mut self, other: &Self) -> bool {
        let before = [
            self.owners.len(),
            self.closures.len(),
            self.homes.len(),
            self.types.len(),
            self.boxed_homes.len(),
        ];
        let changed = other.unknown_callable && !self.unknown_callable
            || other.unknown_contents && !self.unknown_contents;
        for owner in &other.owners {
            if !self.owners.contains(owner) {
                self.owners.insert(owner.summarized(self.owners.iter()));
            }
        }
        for closure in &other.closures {
            if self.closures.contains(closure) {
                continue;
            }
            let mut closure = closure.clone();
            closure.home = closure.home.summarized(
                self.closures
                    .iter()
                    .filter(|old| {
                        old.creator == closure.creator && old.descriptor == closure.descriptor
                    })
                    .map(|old| &old.home),
            );
            self.closures.insert(closure);
        }
        self.homes.extend(other.homes.iter().copied());
        self.types.extend(other.types.iter().copied());
        self.boxed_homes.extend(other.boxed_homes.iter().copied());
        self.unknown_callable |= other.unknown_callable;
        self.unknown_contents |= other.unknown_contents;
        changed
            || before
                != [
                    self.owners.len(),
                    self.closures.len(),
                    self.homes.len(),
                    self.types.len(),
                    self.boxed_homes.len(),
                ]
    }

    fn rehome(&mut self, owner: Owner) {
        self.owners = HashSet::from([owner.clone()]);
        self.closures = self
            .closures
            .iter()
            .map(|closure| Closure {
                home: owner.clone(),
                ..closure.clone()
            })
            .collect();
    }

    fn join_on_cycle(&mut self, other: &Self, cyclic: bool) -> bool {
        if !cyclic {
            return self.join(other);
        }
        let mut incoming = other.clone();
        incoming.owners = incoming
            .owners
            .into_iter()
            .map(|mut owner| {
                owner.widen_on_cycle(self.owners.iter());
                owner
            })
            .collect();
        incoming.closures = incoming
            .closures
            .into_iter()
            .map(|mut closure| {
                closure.home.widen_on_cycle(
                    self.closures
                        .iter()
                        .filter(|old| {
                            old.creator == closure.creator && old.descriptor == closure.descriptor
                        })
                        .map(|old| &old.home),
                );
                closure
            })
            .collect();
        self.join(&incoming)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Selection {
    request: Root,
    storage: Owner,
    writable: bool,
}

impl Selection {
    fn validate(&self) -> Result<(), BackendError> {
        if !matches!(self.request, Root::Request(_, _))
            || !(self.storage.root == Root::Group
                || self.storage.root == Root::Adapter && !self.storage.fields.is_empty())
        {
            return Err(malformed_mir(
                "pending request retains callback-local, consumed or unresolved storage",
            ));
        }
        Ok(())
    }
}

/// One origin value for each local of the function being proved.
///
/// The proof keeps one copy of this state for every block it has reached and
/// makes another for every edge it follows. A block changes a handful of
/// locals, so the copies share every value they have not changed: copying
/// costs a reference count for each local, and joining two copies skips the
/// values they still share.
#[derive(Clone, PartialEq, Eq)]
struct LocalValues(Vec<std::rc::Rc<Value>>);

impl LocalValues {
    fn new(count: usize) -> Self {
        // Every local starts as the same empty value, held once.
        let empty = std::rc::Rc::new(Value::default());
        Self((0..count).map(|_| std::rc::Rc::clone(&empty)).collect())
    }

    #[cfg(test)]
    fn push(&mut self, value: Value) {
        self.0.push(std::rc::Rc::new(value));
    }

    fn join(&mut self, incoming: &Self, cyclic: bool) -> bool {
        let mut changed = false;
        for (value, incoming) in self.0.iter_mut().zip(&incoming.0) {
            // Joining a value with itself changes nothing, except that a join
            // across a back edge first widens each owner against the others
            // and each closure home against the other homes. One owner and
            // one closure have nothing to widen against.
            if std::rc::Rc::ptr_eq(value, incoming)
                && (!cyclic || value.owners.len() <= 1 && value.closures.len() <= 1)
            {
                continue;
            }
            changed |= std::rc::Rc::make_mut(value).join_on_cycle(incoming, cyclic);
        }
        changed
    }

    /// Applies `change` to every value `matches` selects and leaves the
    /// others shared.
    fn change_where(&mut self, matches: impl Fn(&Value) -> bool, change: impl Fn(&mut Value)) {
        for value in &mut self.0 {
            if matches(value) {
                change(std::rc::Rc::make_mut(value));
            }
        }
    }
}

impl From<Vec<Value>> for LocalValues {
    fn from(values: Vec<Value>) -> Self {
        Self(values.into_iter().map(std::rc::Rc::new).collect())
    }
}

impl std::ops::Index<usize> for LocalValues {
    type Output = Value;

    fn index(&self, local: usize) -> &Value {
        &self.0[local]
    }
}

impl std::ops::IndexMut<usize> for LocalValues {
    fn index_mut(&mut self, local: usize) -> &mut Value {
        std::rc::Rc::make_mut(&mut self.0[local])
    }
}

// Function-local loan safety is checked before this call-context proof;
// retain only the interprocedural origin facts here.
#[derive(Clone, PartialEq, Eq)]
struct Flow {
    values: LocalValues,
    fields: HashMap<Owner, Value>,
    selected: HashSet<Selection>,
    writes: HashMap<Owner, Value>,
}

impl Flow {
    fn store_projected(&mut self, destination: Owner, value: &Value, relocate: bool) {
        if relocate {
            let mut stored = value.clone();
            self.rehome(&mut stored, destination.clone());
            self.fields.entry(destination).or_default().join(&stored);
        } else {
            self.fields.entry(destination).or_default().join(value);
        }
    }

    fn join(&mut self, incoming: Self, cyclic: bool) -> bool {
        let mut changed = self.values.join(&incoming.values, cyclic);
        changed |= join_storage(&mut self.fields, &incoming.fields, cyclic);
        changed |= join_storage(&mut self.writes, &incoming.writes, cyclic);
        let before = self.selected.len();
        self.selected.extend(incoming.selected);
        changed | (self.selected.len() != before)
    }

    fn field(&self, selected: &Owner) -> Option<Value> {
        let mut value = None::<Value>;
        for (field, stored) in &self.fields {
            if field.storage_overlaps(selected) {
                value.get_or_insert_with(Value::default).join(stored);
            }
        }
        value
    }

    fn rehome(&mut self, value: &mut Value, destination: Owner) {
        Self::rehome_fields(&mut self.fields, value, destination);
    }

    fn rehome_fields(
        existing: &mut HashMap<Owner, Value>,
        value: &mut Value,
        destination: Owner,
    ) -> bool {
        let sources = value.owners.clone();
        let mut fields = HashMap::new();
        for (owner, field) in existing.iter() {
            for source in &sources {
                if owner.root == source.root && owner.fields.starts_with(&source.fields) {
                    let mut moved = owner.clone();
                    moved.relocate(source, &destination);
                    let mut field = field.clone();
                    field.owners = field
                        .owners
                        .into_iter()
                        .map(|mut owner| {
                            owner.relocate(source, &destination);
                            owner
                        })
                        .collect();
                    field.closures = field
                        .closures
                        .into_iter()
                        .map(|mut closure| {
                            closure.home.relocate(source, &destination);
                            closure
                        })
                        .collect();
                    fields
                        .entry(moved)
                        .or_insert_with(Value::default)
                        .join(&field);
                }
            }
        }
        let changed = join_fields(existing, &fields);
        value.rehome(destination);
        changed
    }

    fn write(&mut self, previous: &Value, replacement: &Value) {
        for owner in &previous.owners {
            self.writes
                .entry(owner.clone())
                .or_default()
                .join(replacement);
            self.values.change_where(
                |alias| {
                    alias
                        .owners
                        .iter()
                        .any(|alias| alias.storage_overlaps(owner))
                },
                |alias| {
                    alias.types.extend(replacement.types.iter().copied());
                    alias.closures.extend(replacement.closures.iter().cloned());
                    alias
                        .boxed_homes
                        .extend(replacement.boxed_homes.iter().copied());
                    alias.unknown_callable |= replacement.unknown_callable;
                    alias.unknown_contents |= replacement.unknown_contents;
                },
            );
            for alias in self.fields.values_mut() {
                if alias
                    .owners
                    .iter()
                    .any(|alias| alias.storage_overlaps(owner))
                {
                    alias.types.extend(replacement.types.iter().copied());
                    alias.closures.extend(replacement.closures.iter().cloned());
                    alias
                        .boxed_homes
                        .extend(replacement.boxed_homes.iter().copied());
                    alias.unknown_callable |= replacement.unknown_callable;
                    alias.unknown_contents |= replacement.unknown_contents;
                }
            }
        }
    }
}

fn join_writes(destination: &mut HashMap<Owner, Value>, incoming: &HashMap<Owner, Value>) -> bool {
    join_storage(destination, incoming, false)
}

fn join_fields(destination: &mut HashMap<Owner, Value>, incoming: &HashMap<Owner, Value>) -> bool {
    join_storage(destination, incoming, false)
}

fn join_storage(
    destination: &mut HashMap<Owner, Value>,
    incoming: &HashMap<Owner, Value>,
    cyclic: bool,
) -> bool {
    let before = destination.len();
    let mut changed = false;
    for (field, value) in incoming {
        if let Some(stored) = destination.get_mut(field) {
            changed |= stored.join_on_cycle(value, cyclic);
            continue;
        }
        let mut field = field.clone();
        if cyclic {
            field.widen_on_cycle(destination.keys());
        }
        let field = field.summarized(destination.keys());
        changed |= destination
            .entry(field)
            .or_default()
            .join_on_cycle(value, cyclic);
    }
    changed || destination.len() != before
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Site {
    block: mir::BlockId,
    statement: usize,
    call: usize,
}

#[derive(Default)]
pub(super) struct CallbackOrigin {
    pub(super) unknown: bool,
    pub(super) targets: HashSet<mir::FunctionId>,
}

pub(super) type CallbackOrigins = HashMap<(mir::FunctionId, mir::BlockId), CallbackOrigin>;

fn value_call_count(value: &mir::Rvalue) -> usize {
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    accesses.direct_calls
}

fn value_site(
    site: Site,
    accesses: &ClassLocalAccesses<'_>,
    value: &mir::Rvalue,
) -> Result<Site, BackendError> {
    // Identity only locates the actual operand in this traversal. Provenance
    // is keyed by the resulting canonical ordinal, never by argument equality
    // or by the callee's name. Typed wrapper clones inherit that ordinal.
    let call = accesses
        .value_call_starts
        .iter()
        .find_map(|(operand, call)| std::ptr::eq(*operand, value).then_some(*call))
        .ok_or_else(|| malformed_mir("pending operand has no canonical evaluation position"))?;
    Ok(Site { call, ..site })
}

/// Storage and closure creators are named by scope, relative to the root of
/// the context tree that observes them: a writeback or pending root, or a call
/// summary. `Within` names the frame one call step below its parent, so names
/// inside one tree are its call paths. A summary names every caller frame in
/// its entry `Foreign`, so calls with equal entries share one analysis.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Scope {
    Own,
    Within(usize, Step),
    Foreign(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Step {
    Call(Site, mir::FunctionId, Option<Closure>),
    Definition(mir::ClosureDescriptorId),
}

const OWN: usize = 0;

struct Scopes {
    entries: Vec<Scope>,
    ids: HashMap<Scope, usize>,
}

impl Scopes {
    fn new() -> Self {
        let mut scopes = Self {
            entries: Vec::new(),
            ids: HashMap::new(),
        };
        scopes.intern(Scope::Own);
        scopes
    }

    fn intern(&mut self, scope: Scope) -> usize {
        if let Some(id) = self.ids.get(&scope) {
            return *id;
        }
        let id = self.entries.len();
        self.entries.push(scope.clone());
        self.ids.insert(scope, id);
        id
    }

    fn get(&self, id: usize) -> &Scope {
        &self.entries[id]
    }

    fn under(&self, mut scope: usize, frame: usize) -> bool {
        loop {
            if scope == frame {
                return true;
            }
            match &self.entries[scope] {
                Scope::Within(parent, _) => scope = *parent,
                _ => return false,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Root,
    Child,
    Summary,
    Definition,
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct Context {
    function: mir::FunctionId,
    parent: Option<usize>,
    kind: Kind,
    // This context's own frame, relative to `root`, the root of its tree.
    scope: usize,
    root: usize,
    dirty: bool,
    inputs: Vec<Value>,
    captures: HashMap<mir::LocalId, Value>,
    fields: HashMap<Owner, Value>,
    selected: HashSet<Selection>,
    // A summary's canonical entry, including its caller closures' environments.
    entry: Option<Entry>,
    returned: Value,
    normal_return: bool,
    outgoing: HashSet<Selection>,
    outgoing_fields: HashMap<Owner, Value>,
    outgoing_values: HashMap<Owner, Value>,
    // Advances whenever the exit above changes, so instantiations stay current.
    version: usize,
}

impl Context {
    fn new(
        function: mir::FunctionId,
        parent: Option<usize>,
        kind: Kind,
        scope: usize,
        root: usize,
        inputs: Vec<Value>,
    ) -> Self {
        Self {
            function,
            parent,
            kind,
            scope,
            root,
            dirty: true,
            inputs,
            captures: HashMap::new(),
            fields: HashMap::new(),
            selected: HashSet::new(),
            entry: None,
            returned: Value::default(),
            normal_return: false,
            outgoing: HashSet::new(),
            outgoing_fields: HashMap::new(),
            outgoing_values: HashMap::new(),
            version: 0,
        }
    }
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct CallableDefinition {
    // The creator's facts and the closure, both in the creator's naming.
    closure: Closure,
    fields: HashMap<Owner, Value>,
    context: Option<usize>,
}

/// A call entry in callee naming. Fields and selections are restricted to
/// storage the callee can reach: storage is only ever matched by root, and the
/// caller replays the callee's writes to its own aliases.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    function: mir::FunctionId,
    closure: Option<Closure>,
    inputs: Vec<Value>,
    captures: HashMap<mir::LocalId, Value>,
    fields: HashMap<Owner, Value>,
    selected: HashSet<Selection>,
    environments: HashMap<(usize, mir::ClosureDescriptorId), Vec<Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Context(usize),
    Record(usize),
}

/// A call that crosses a strongly connected component of the call graph.
/// It accumulates its caller-view entry exactly as a path context would, then
/// binds to the summary for that entry's canonical form.
struct Record {
    caller: usize,
    function: mir::FunctionId,
    closure: Option<Closure>,
    // The callee's own frame in caller naming.
    frame: usize,
    inputs: Vec<Value>,
    captures: HashMap<mir::LocalId, Value>,
    fields: HashMap<Owner, Value>,
    selected: HashSet<Selection>,
    summary: Option<usize>,
    // Caller scope and caller storage named by each `Foreign` scope and root.
    sigma: Vec<usize>,
    sigma_roots: Vec<Root>,
    rebased: HashMap<usize, usize>,
    exit: Option<(usize, std::rc::Rc<Exit>)>,
}

struct Exit {
    returned: Value,
    outgoing: HashSet<Selection>,
    outgoing_fields: HashMap<Owner, Value>,
    outgoing_values: HashMap<Owner, Value>,
}

struct Proof<'a> {
    program: &'a mir::Program,
    purpose: Purpose,
    contexts: Vec<Context>,
    calls: HashMap<(usize, Site, mir::FunctionId, Option<Closure>), Call>,
    // Keyed by the creating context; values use that context's tree naming.
    environments: HashMap<(usize, mir::ClosureDescriptorId), Vec<Value>>,
    definitions: HashMap<(usize, mir::ClosureDescriptorId), CallableDefinition>,
    callback_origins: CallbackOrigins,
    changed: bool,
    scopes: Scopes,
    // Strongly connected component of each function, including closure calls.
    components: Vec<usize>,
    records: Vec<Record>,
    summaries: HashMap<(mir::FunctionId, u64), Vec<usize>>,
    // Contexts to revisit when a callee's exit or a read environment changes.
    dependents: HashMap<usize, HashSet<usize>>,
    environment_readers: HashMap<(usize, mir::ClosureDescriptorId), HashSet<usize>>,
    record_readers: HashMap<usize, HashSet<usize>>,
}

struct Canonical {
    frame: usize,
    assigned: HashMap<usize, usize>,
    foreign: Vec<usize>,
    roots: HashMap<Root, Root>,
    foreign_roots: Vec<Root>,
    reached: HashSet<Root>,
    frontier: std::collections::VecDeque<Root>,
    closures: HashSet<(usize, mir::ClosureDescriptorId)>,
    pending: std::collections::VecDeque<Closure>,
}

/// Renames the scopes and storage roots of proof facts between namings.
trait Rename {
    fn scope(&mut self, scope: usize) -> Option<usize>;

    fn root(&mut self, root: Root) -> Option<Root> {
        rename_root_scope(root, self)
    }
}

// Storage named by caller scope, or by a summary's `Foreign` root, moves as a
// unit. Global roots have one meaning in every tree.
fn root_is_scoped(root: Root) -> bool {
    matches!(
        root,
        Root::Local(..)
            | Root::Temporary(..)
            | Root::Construction(..)
            | Root::Environment(..)
            | Root::Foreign(_)
    )
}

fn root_scope(root: Root) -> Option<usize> {
    match root {
        Root::Local(scope, _)
        | Root::Temporary(scope, _)
        | Root::Construction(scope, _, _)
        | Root::Environment(scope, _) => Some(scope),
        _ => None,
    }
}

fn rename_root_scope<R: Rename + ?Sized>(root: Root, rename: &mut R) -> Option<Root> {
    Some(match root {
        Root::Local(scope, local) => Root::Local(rename.scope(scope)?, local),
        Root::Temporary(scope, site) => Root::Temporary(rename.scope(scope)?, site),
        Root::Construction(scope, site, span) => {
            Root::Construction(rename.scope(scope)?, site, span)
        }
        Root::Environment(scope, descriptor) => Root::Environment(rename.scope(scope)?, descriptor),
        Root::Adapter
        | Root::Group
        | Root::Request(..)
        | Root::Static(_)
        | Root::Unknown
        | Root::Foreign(_) => root,
    })
}

/// Removes every naming-dependent part, leaving only a fact's shape.
struct Erase;

impl Rename for Erase {
    fn scope(&mut self, _: usize) -> Option<usize> {
        Some(usize::MAX)
    }

    fn root(&mut self, root: Root) -> Option<Root> {
        Some(if root_is_scoped(root) {
            Root::Unknown
        } else {
            root
        })
    }
}

struct Canonicalize<'p, 'a, 's> {
    proof: &'p mut Proof<'a>,
    state: &'s mut Canonical,
}

impl Rename for Canonicalize<'_, '_, '_> {
    fn scope(&mut self, scope: usize) -> Option<usize> {
        self.proof.canonical_scope(self.state, scope)
    }

    fn root(&mut self, root: Root) -> Option<Root> {
        self.proof.canonical_root(self.state, root)
    }
}

struct Rebase<'p, 'a> {
    proof: &'p mut Proof<'a>,
    record: usize,
}

impl Rename for Rebase<'_, '_> {
    fn scope(&mut self, scope: usize) -> Option<usize> {
        self.proof.rebase_scope(self.record, scope)
    }

    fn root(&mut self, root: Root) -> Option<Root> {
        match root {
            Root::Foreign(index) => self.proof.records[self.record]
                .sigma_roots
                .get(index)
                .copied(),
            _ => rename_root_scope(root, self),
        }
    }
}

struct Unrebase<'p, 'a> {
    proof: &'p mut Proof<'a>,
    record: usize,
}

impl Rename for Unrebase<'_, '_> {
    fn scope(&mut self, scope: usize) -> Option<usize> {
        self.proof.unrebase_scope(self.record, scope)
    }

    fn root(&mut self, root: Root) -> Option<Root> {
        self.proof.unrebase_root(self.record, root)
    }
}

fn map_owner(owner: &Owner, rename: &mut dyn Rename) -> Option<Owner> {
    Some(Owner {
        root: rename.root(owner.root)?,
        ..owner.clone()
    })
}

fn map_closure(closure: &Closure, rename: &mut dyn Rename) -> Option<Closure> {
    Some(Closure {
        creator: rename.scope(closure.creator)?,
        descriptor: closure.descriptor,
        home: map_owner(&closure.home, rename)?,
    })
}

fn map_step(step: &Step, rename: &mut dyn Rename) -> Option<Step> {
    Some(match step {
        Step::Call(site, function, closure) => Step::Call(
            *site,
            *function,
            match closure {
                Some(closure) => Some(map_closure(closure, rename)?),
                None => None,
            },
        ),
        Step::Definition(descriptor) => Step::Definition(*descriptor),
    })
}

// Built directly: a join would summarize owners that the source kept apart.
fn map_value(value: &Value, rename: &mut dyn Rename) -> Option<Value> {
    Some(Value {
        owners: value
            .owners
            .iter()
            .map(|owner| map_owner(owner, rename))
            .collect::<Option<_>>()?,
        closures: value
            .closures
            .iter()
            .map(|closure| map_closure(closure, rename))
            .collect::<Option<_>>()?,
        ..value.clone()
    })
}

fn map_storage(
    storage: &HashMap<Owner, Value>,
    rename: &mut dyn Rename,
) -> Option<HashMap<Owner, Value>> {
    let mut mapped = HashMap::with_capacity(storage.len());
    for (owner, value) in storage {
        let value = map_value(value, rename)?;
        match mapped.entry(map_owner(owner, rename)?) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(value);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                slot.get_mut().join(&value);
            }
        }
    }
    Some(mapped)
}

fn map_selection(selection: &Selection, rename: &mut dyn Rename) -> Option<Selection> {
    Some(Selection {
        request: rename.root(selection.request)?,
        storage: map_owner(&selection.storage, rename)?,
        writable: selection.writable,
    })
}

fn apply_exit(
    purpose: Purpose,
    outgoing: &HashSet<Selection>,
    fields: &HashMap<Owner, Value>,
    values: &HashMap<Owner, Value>,
    flow: &mut Flow,
    checking: bool,
) -> Result<(), BackendError> {
    if checking && purpose == Purpose::Pending {
        // Diagnose the originating selection before its caller's cleanup
        // can obscure it with a downstream lifetime conflict.
        for selection in outgoing {
            selection.validate()?;
        }
    }
    flow.selected.extend(outgoing.iter().cloned());
    join_fields(&mut flow.fields, fields);
    for (owner, value) in values {
        flow.write(
            &Value {
                owners: HashSet::from([owner.clone()]),
                ..Value::default()
            },
            value,
        );
    }
    Ok(())
}

fn hash_one(value: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    // Fixed keys: canonical numbering must not depend on per-map hash seeds.
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn unordered(hashes: impl Iterator<Item = u64>) -> u64 {
    hashes.fold(0, u64::wrapping_add)
}

// Naming orders only equal shapes, which costs sharing, never precision.
fn owner_shape(owner: &Owner) -> (u64, u64) {
    let erased = map_owner(owner, &mut Erase).unwrap_or_else(|| owner.clone());
    (hash_one(&erased), hash_one(owner))
}

fn closure_shape(closure: &Closure) -> (u64, u64) {
    let erased = map_closure(closure, &mut Erase).unwrap_or_else(|| closure.clone());
    (hash_one(&erased), hash_one(closure))
}

fn value_fingerprint(value: &Value) -> u64 {
    hash_one(&(
        unordered(value.owners.iter().map(hash_one)),
        unordered(value.closures.iter().map(hash_one)),
        unordered(value.homes.iter().map(hash_one)),
        unordered(value.types.iter().map(hash_one)),
        unordered(value.boxed_homes.iter().map(hash_one)),
        value.unknown_callable,
        value.unknown_contents,
    ))
}

fn entry_fingerprint(entry: &Entry) -> u64 {
    hash_one(&(
        entry.function,
        &entry.closure,
        entry
            .inputs
            .iter()
            .map(value_fingerprint)
            .collect::<Vec<_>>(),
        unordered(
            entry
                .captures
                .iter()
                .map(|(local, value)| hash_one(&(local, value_fingerprint(value)))),
        ),
        unordered(
            entry
                .fields
                .iter()
                .map(|(owner, value)| hash_one(&(owner, value_fingerprint(value)))),
        ),
        unordered(entry.selected.iter().map(hash_one)),
        unordered(entry.environments.iter().map(|(key, values)| {
            hash_one(&(
                key,
                values.iter().map(value_fingerprint).collect::<Vec<_>>(),
            ))
        })),
    ))
}

pub(super) fn validate_pending(program: &mir::Program) -> Result<(), BackendError> {
    // No AST walk, reflection, adapter spelling heuristic or source effect plan.
    let entries = preparation_entries(program)?;
    for function in entries {
        let definition = function_in(program, function)?;
        let mut inputs = vec![Value::default(); definition.params.len()];
        inputs[0] = Value::owner(Root::Adapter);
        inputs[1] = Value::owner(Root::Request(function, definition.params[1]));
        inputs[2] = Value::owner(Root::Group);
        let mut proof = Proof::new(program, Purpose::Pending)?;
        proof
            .contexts
            .push(Context::new(function, None, Kind::Root, OWN, 0, inputs));
        while proof.changed {
            proof.changed = false;
            proof.pass()?;
        }
        proof.check()?;
    }
    Ok(())
}

pub(super) fn validate_calls(program: &mir::Program) -> Result<CallbackOrigins, BackendError> {
    Ok(writeback::validate(program)?.callback_origins)
}

fn preparation_entries(program: &mir::Program) -> Result<HashSet<mir::FunctionId>, BackendError> {
    use crate::compiler_known_stream::StreamOperation;
    let mut entries = HashSet::new();
    for table in &program.interface_vtables {
        let interface = interface_in(program, table.interface)?;
        for (operation, reading) in [
            (StreamOperation::PrepareRead, true),
            (StreamOperation::PrepareWrite, false),
        ] {
            let Some(callback) =
                program.stream_callback(mir::Type::Interface(table.interface), operation)
            else {
                continue;
            };
            let method = &interface.methods[callback.slot];
            let entry = *table
                .methods
                .get(callback.slot)
                .ok_or_else(|| malformed_mir("pending preparation vtable entry is missing"))?;
            let function = function_in(program, entry)?;
            let signature = function_type_in(program, callback.signature)?;
            if function.params.len() != 3
                || signature.return_type != mir::ReturnType::Void
                || method.receiver_mode != mir::ReceiverMode::Writable
            {
                return Err(malformed_mir(
                    "pending preparation has an invalid nominal boundary",
                ));
            }
            let request = local_in(function, function.params[1])?;
            if validate_pending_request(program, function, request.id, true)? != reading {
                return Err(malformed_mir(
                    "pending preparation request direction differs from its nominal requirement",
                ));
            }
            let region = local_in(function, function.params[2])?;
            let kind = if reading {
                mir::ByteRegionKind::Writable
            } else {
                mir::ByteRegionKind::Readonly
            };
            if region.ty != mir::Type::ByteRegion(kind)
                || function.parameter_modes[0] != mir::FunctionParameterMode::Writable
                || function.parameter_modes[1] != mir::FunctionParameterMode::Writable
                || function.parameter_modes[2]
                    != if reading {
                        mir::FunctionParameterMode::Writable
                    } else {
                        mir::FunctionParameterMode::Readonly
                    }
            {
                return Err(malformed_mir(
                    "pending preparation inputs disagree with the canonical request/storage contract",
                ));
            }
            entries.insert(entry);
        }
    }
    Ok(entries)
}

impl<'a> Proof<'a> {
    fn new(program: &'a mir::Program, purpose: Purpose) -> Result<Self, BackendError> {
        Ok(Self {
            program,
            purpose,
            contexts: Vec::new(),
            calls: HashMap::new(),
            environments: HashMap::new(),
            definitions: HashMap::new(),
            callback_origins: HashMap::new(),
            changed: true,
            scopes: Scopes::new(),
            components: writeback::components(program)?,
            records: Vec::new(),
            summaries: HashMap::new(),
            dependents: HashMap::new(),
            environment_readers: HashMap::new(),
            record_readers: HashMap::new(),
        })
    }
}

impl Proof<'_> {
    /// Contexts reachable from the roots through current calls and definition
    /// proofs. A record that re-binds abandons a summary for a smaller entry;
    /// such a summary is neither revisited, counted as coverage, nor checked.
    fn live(&self) -> Vec<bool> {
        let mut successors = vec![Vec::new(); self.contexts.len()];
        for ((caller, ..), call) in &self.calls {
            let callee = match call {
                Call::Context(id) => Some(*id),
                Call::Record(record) => self.records[*record].summary,
            };
            successors[*caller].extend(callee);
        }
        for ((creator, _), definition) in &self.definitions {
            successors[*creator].extend(definition.context);
        }
        let mut live = vec![false; self.contexts.len()];
        let mut pending = (0..self.contexts.len())
            .filter(|context| self.contexts[*context].kind == Kind::Root)
            .collect::<Vec<_>>();
        while let Some(context) = pending.pop() {
            if !std::mem::replace(&mut live[context], true) {
                pending.extend(successors[context].iter().copied());
            }
        }
        live
    }

    /// Visits every live dirty context once. Contexts created during the pass
    /// are reached from a live caller and are visited in the same pass.
    fn pass(&mut self) -> Result<(), BackendError> {
        let live = self.live();
        for context in 0..self.contexts.len() {
            if live.get(context).copied().unwrap_or(true) {
                self.body(context, false)?;
            }
        }
        Ok(())
    }

    fn check(&mut self) -> Result<(), BackendError> {
        let live = self.live();
        for context in 0..self.contexts.len() {
            if live.get(context).copied().unwrap_or(true) {
                self.body(context, true)?;
            }
        }
        Ok(())
    }

    /// Absolute creators of closures executed by a live call. A summary's
    /// foreign closure was created wherever each live caller's closure was.
    fn invoked_closures(
        &mut self,
        live: &[bool],
    ) -> Result<HashSet<(usize, mir::ClosureDescriptorId)>, BackendError> {
        let bound = self
            .calls
            .keys()
            .filter(|(caller, ..)| live[*caller])
            .filter_map(|(caller, _, _, closure)| {
                closure
                    .as_ref()
                    .map(|closure| (*caller, closure.creator, closure.descriptor))
            })
            .collect::<Vec<_>>();
        let mut invoked = HashSet::new();
        for (caller, creator, descriptor) in bound {
            let root = self.contexts[caller].root;
            for creator in self.creators(root, creator, live)? {
                invoked.insert((creator, descriptor));
            }
        }
        Ok(invoked)
    }

    fn creators(
        &mut self,
        root: usize,
        scope: usize,
        live: &[bool],
    ) -> Result<Vec<usize>, BackendError> {
        if let Scope::Foreign(index) = self.scopes.get(scope) {
            let index = *index;
            let callers = self
                .records
                .iter()
                .filter(|record| record.summary == Some(root) && live[record.caller])
                .map(|record| (self.contexts[record.caller].root, record.sigma[index]))
                .collect::<Vec<_>>();
            let mut creators = Vec::new();
            for (root, scope) in callers {
                creators.extend(self.creators(root, scope, live)?);
            }
            return Ok(creators);
        }
        Ok(self
            .frame(root, scope)?
            .map(|(context, _)| context)
            .into_iter()
            .collect())
    }

    fn seed_callable_definitions(&mut self) -> Result<bool, BackendError> {
        let live = self.live();
        let invoked = self.invoked_closures(&live)?;
        let mut definitions = self.definitions.keys().copied().collect::<Vec<_>>();
        definitions.sort_unstable_by_key(|(creator, descriptor)| (*creator, descriptor.0));
        let mut changed = false;
        for key in definitions {
            let (creator, descriptor) = key;
            if !live[creator] || invoked.contains(&key) {
                continue;
            }
            let closure = self.definitions[&key].closure.clone();
            let fields = self.definitions[&key].fields.clone();
            let id = if let Some(id) = self.definitions[&key].context {
                id
            } else {
                // A definition proof checks a real captured environment without
                // pretending its construction executes or affects the creator.
                let function = closure_descriptor_in(self.program, descriptor)?.entry_function;
                let definition = function_in(self.program, function)?;
                let id = self.contexts.len();
                let scope = self.scopes.intern(Scope::Within(
                    self.contexts[creator].scope,
                    Step::Definition(descriptor),
                ));
                let inputs = definition
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, local)| {
                        if index == 0 {
                            Value {
                                owners: HashSet::from([closure.home.clone()]),
                                closures: HashSet::from([closure.clone()]),
                                ..Value::default()
                            }
                        } else {
                            let ty = definition.locals[local.0].ty;
                            let mut value = Value::owner(Root::Local(scope, Some(*local)));
                            value.homes.insert(Some(ty));
                            value.types.insert(Some(ty));
                            value.mark_unknown_callable(ty);
                            value
                        }
                    })
                    .collect();
                let root = self.contexts[creator].root;
                self.contexts.push(Context::new(
                    function,
                    Some(creator),
                    Kind::Definition,
                    scope,
                    root,
                    inputs,
                ));
                self.definitions
                    .get_mut(&key)
                    .expect("recorded definition")
                    .context = Some(id);
                changed = true;
                id
            };
            let fields_changed = join_fields(&mut self.contexts[id].fields, &fields);
            self.contexts[id].dirty |= fields_changed;
            changed |= fields_changed;
            self.bind_closure(id, &closure, false, id)?;
        }
        self.changed |= changed;
        Ok(self.changed)
    }

    fn body(&mut self, context: usize, checking: bool) -> Result<(), BackendError> {
        if !checking {
            if !self.contexts[context].dirty {
                return Ok(());
            }
            // Recursive calls may dirty this context again during its visit.
            self.contexts[context].dirty = false;
        }
        let program = self.program;
        let function = function_in(program, self.contexts[context].function)?;
        let own = self.contexts[context].scope;
        let finalizers = super::FinalizerExits::of(function);
        let mut pending = borrowed_views::NodeWorklist::new(function)?;
        let mut initial = Flow {
            values: LocalValues::new(function.locals.len()),
            fields: self.contexts[context].fields.clone(),
            selected: self.contexts[context].selected.clone(),
            writes: HashMap::new(),
        };
        for (parameter, value) in function.params.iter().zip(&self.contexts[context].inputs) {
            let mut value = value.clone();
            if local_in(function, *parameter)?.owned
                && !self.is_region_local(context, *parameter)?
                && !value.owners.is_empty()
            {
                initial.rehome(&mut value, Owner::new(Root::Local(own, Some(*parameter))));
                if self.purpose == Purpose::Writeback {
                    value.homes = HashSet::from([Some(function.locals[parameter.0].ty)]);
                }
            }
            if self.purpose == Purpose::Writeback
                && !function.locals[parameter.0].ty.has_move_ownership()
                && !function.is_writable_borrowed_place(*parameter)
            {
                value.homes = HashSet::from([Some(function.locals[parameter.0].ty)]);
            }
            initial.values[parameter.0] = value;
        }
        for (local, value) in &self.contexts[context].captures {
            initial.values[local.0] = value.clone();
        }
        let mut entries = HashMap::<borrowed_views::Node, Flow>::new();
        entries.insert((function.entry_block, Vec::new()), initial);
        let mut returned = Value::default();
        let mut normal_return = false;
        let mut outgoing = HashSet::<Selection>::new();
        let mut fields = HashMap::<Owner, Value>::new();
        let mut writes = HashMap::new();
        while let Some(node) = pending.pop() {
            let id = node.0;
            let block = block_in(function, id)?;
            let mut flow = entries[&node].clone();
            for (index, statement) in block.statements.iter().enumerate() {
                let site = Site {
                    block: id,
                    statement: index,
                    call: 0,
                };
                self.statement(context, site, &mut flow, statement, false)?;
            }
            let site = Site {
                block: id,
                statement: block.statements.len(),
                call: 0,
            };
            self.calls_in(
                context,
                site,
                &mut flow,
                &collect_terminator_class_local_accesses(&block.terminator),
                false,
            )?;
            let call_result = match &block.terminator {
                mir::Terminator::IndirectCall { callee, args, .. }
                | mir::Terminator::CheckedIndirectCall { callee, args, .. } => {
                    Some(self.indirect(context, site, &mut flow, callee, args, false)?)
                }
                _ => None,
            };
            match &block.terminator {
                mir::Terminator::Return(value) => {
                    normal_return = true;
                    returned.join(&self.aggregate_value(context, site, &mut flow, value)?);
                }
                mir::Terminator::ReturnVoid => normal_return = true,
                mir::Terminator::PropagateError { .. } => {}
                _ => {}
            }
            if matches!(
                block.terminator,
                mir::Terminator::Return(_)
                    | mir::Terminator::ReturnVoid
                    | mir::Terminator::PropagateError { .. }
            ) {
                outgoing.extend(flow.selected.iter().cloned());
                join_fields(&mut fields, &flow.fields);
                join_writes(&mut writes, &flow.writes);
            }
            let infeasible = self.absent_branch(context, site, &flow, &block.terminator)?;
            for successor in borrowed_views::node_successors(&finalizers, block, &node.1) {
                let target = successor.0;
                if infeasible == Some(target) {
                    continue;
                }
                if let mir::Terminator::CheckedCall {
                    function: callee,
                    args,
                    success,
                    ..
                } = &block.terminator
                {
                    // A converged always-throwing call has no success value.
                    // Keep its failure edge and all storage checks intact.
                    if *success == target
                        && !self.direct_call_may_return(context, site, *callee, args)?
                    {
                        continue;
                    }
                }
                let mut edge = flow.clone();
                match &block.terminator {
                    mir::Terminator::CheckedIo {
                        operation:
                            mir::CheckedIoOperation::PreparePendingFile {
                                request, region, ..
                            }
                            | mir::CheckedIoOperation::PreparePendingStandard {
                                request, region, ..
                            },
                        success,
                        ..
                    } if *success == target && self.purpose == Purpose::Pending => {
                        self.select_region(context, &mut edge, *request, *region, false)?;
                    }
                    mir::Terminator::CheckedCall {
                        function: callee,
                        args,
                        result: Some(result),
                        success,
                        ..
                    } if *success == target => {
                        edge.values[result.0] =
                            self.direct_result(context, site, *callee, args, &flow)?;
                    }
                    mir::Terminator::IndirectCall {
                        result: Some(result),
                        continuation,
                        ..
                    } if *continuation == target => {
                        edge.values[result.0] = call_result.clone().unwrap_or_default();
                    }
                    mir::Terminator::CheckedIndirectCall {
                        result: Some(result),
                        success,
                        ..
                    } if *success == target => {
                        edge.values[result.0] = call_result.clone().unwrap_or_default();
                    }
                    mir::Terminator::CheckedConstruct {
                        result,
                        success,
                        class,
                        ..
                    } if *success == target => {
                        edge.values[result.0] = Value::owner(Root::Local(own, Some(*result)));
                        if self.purpose == Purpose::Writeback {
                            edge.values[result.0]
                                .types
                                .insert(Some(mir::Type::Class(*class)));
                            edge.values[result.0]
                                .homes
                                .insert(Some(mir::Type::Class(*class)));
                        }
                    }
                    mir::Terminator::CheckedIo {
                        result: Some(result),
                        success,
                        operation,
                        ..
                    } if *success == target => {
                        edge.values[result.0] = Value::owner(Root::Local(own, Some(*result)));
                        if self.purpose == Purpose::Writeback {
                            let ty = function.locals[result.0].ty;
                            edge.values[result.0].types.insert(Some(ty));
                            edge.values[result.0].homes.insert(Some(match operation {
                                mir::CheckedIoOperation::BorrowReadinessStream {
                                    stream_type,
                                    ..
                                } => *stream_type,
                                _ => ty,
                            }));
                        }
                    }
                    _ => {}
                }
                if self.purpose == Purpose::Writeback {
                    let result = match &block.terminator {
                        mir::Terminator::CheckedCall {
                            result, success, ..
                        }
                        | mir::Terminator::CheckedIndirectCall {
                            result, success, ..
                        } if *success == target => *result,
                        mir::Terminator::IndirectCall {
                            result,
                            continuation,
                            ..
                        } if *continuation == target => *result,
                        _ => None,
                    };
                    if let Some(result) = result {
                        let previous = flow.values[result.0].clone();
                        writeback::assign(
                            program,
                            function,
                            result,
                            &previous,
                            &mut edge.values[result.0],
                            false,
                        )?;
                        let mut value = edge.values[result.0].clone();
                        self.store_local(context, &mut edge, result, &mut value)?;
                        edge.values[result.0] = value;
                    }
                }
                let changed = match entries.get_mut(&successor) {
                    Some(existing) => existing.join(edge, pending.is_back_edge(id, target)),
                    None => {
                        entries.insert(successor.clone(), edge);
                        true
                    }
                };
                if changed {
                    pending.push(successor);
                }
            }
        }
        // Diagnose only converged CFG entries, never an intermediate bottom state.
        if checking {
            let mut nodes = entries.iter().collect::<Vec<_>>();
            nodes.sort_by_key(|((block, selections), _)| {
                (block.0, borrowed_views::node_key(selections))
            });
            for ((block_id, _), entry) in nodes {
                let block = block_in(function, *block_id)?;
                let mut flow = entry.clone();
                for (index, statement) in block.statements.iter().enumerate() {
                    let site = Site {
                        block: block.id,
                        statement: index,
                        call: 0,
                    };
                    self.statement(context, site, &mut flow, statement, true)?;
                }
                let site = Site {
                    block: block.id,
                    statement: block.statements.len(),
                    call: 0,
                };
                self.calls_in(
                    context,
                    site,
                    &mut flow,
                    &collect_terminator_class_local_accesses(&block.terminator),
                    true,
                )?;
                match &block.terminator {
                    mir::Terminator::CheckedIo {
                        operation:
                            mir::CheckedIoOperation::PreparePendingFile {
                                request, region, ..
                            }
                            | mir::CheckedIoOperation::PreparePendingStandard {
                                request, region, ..
                            },
                        ..
                    } if self.purpose == Purpose::Pending => {
                        self.select_region(context, &mut flow, *request, *region, true)?
                    }
                    mir::Terminator::IndirectCall { callee, args, .. }
                    | mir::Terminator::CheckedIndirectCall { callee, args, .. } => {
                        self.indirect(context, site, &mut flow, callee, args, true)?;
                    }
                    _ => {}
                }
            }
        }
        if self.contexts[context].kind == Kind::Summary {
            self.restrict_exit(context, &returned, &mut fields, &mut writes)?;
        }
        let mut changed = self.contexts[context].returned.join(&returned);
        if normal_return && !self.contexts[context].normal_return {
            self.contexts[context].normal_return = true;
            changed = true;
        }
        let before = self.contexts[context].outgoing.len();
        self.contexts[context].outgoing.extend(outgoing);
        changed |= self.contexts[context].outgoing.len() != before;
        changed |= join_fields(&mut self.contexts[context].outgoing_fields, &fields);
        changed |= join_writes(&mut self.contexts[context].outgoing_values, &writes);
        if changed {
            self.changed = true;
            self.contexts[context].version += 1;
            for caller in self.dependents.get(&context).into_iter().flatten() {
                self.contexts[*caller].dirty = true;
            }
        }
        Ok(())
    }

    /// The edge a presence test cannot take because its value is definitely
    /// absent in this context: only the null type, no callback and nothing
    /// unknown. Like a call that never returns, its target is not analyzed,
    /// so a null-only context does not check payload arms that cannot execute.
    fn absent_branch(
        &mut self,
        context: usize,
        site: Site,
        flow: &Flow,
        terminator: &mir::Terminator,
    ) -> Result<Option<mir::BlockId>, BackendError> {
        let mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } = terminator
        else {
            return Ok(None);
        };
        if then_block == else_block {
            return Ok(None);
        }
        let (test, present_edge) = match condition {
            mir::BoolExpression::Not(test) => (test.as_ref(), *else_block),
            test => (test, *then_block),
        };
        use mir::BoolExpression as B;
        let tested = match test {
            B::NullableScalarIsPresent(value) => mir::Rvalue::NullableScalar((**value).clone()),
            B::NullableClassIsPresent(value) => mir::Rvalue::NullableClass((**value).clone()),
            B::NullableCollectionIsPresent(value) => {
                mir::Rvalue::NullableCollection((**value).clone())
            }
            B::NullableByteRegionIsPresent(value) => {
                mir::Rvalue::NullableByteRegion((**value).clone())
            }
            B::NullableSharedReferenceIsPresent(value) => {
                mir::Rvalue::NullableSharedReference((**value).clone())
            }
            B::NullableWeakReferenceIsPresent(value) => {
                mir::Rvalue::NullableWeakReference((**value).clone())
            }
            B::NullableWritableSharedReferenceIsPresent(value) => {
                mir::Rvalue::NullableWritableSharedReference((**value).clone())
            }
            B::NullableWritableWeakReferenceIsPresent(value) => {
                mir::Rvalue::NullableWritableWeakReference((**value).clone())
            }
            B::NullableSharedReferenceAccessIsPresent(value) => {
                mir::Rvalue::NullableSharedReferenceAccess((**value).clone())
            }
            B::NullableMixedIsPresent(value) => mir::Rvalue::NullableMixed((**value).clone()),
            B::NullableErrorIsPresent(value) => mir::Rvalue::NullableInterface((**value).clone()),
            B::NullablePayloadEnumIsPresent(value) => {
                mir::Rvalue::NullablePayloadEnum((**value).clone())
            }
            B::NullableFunctionIsPresent(value) => mir::Rvalue::NullableFunction((**value).clone()),
            _ => return Ok(None),
        };
        // Owners name the slot, not presence: a moved null still has a home.
        let value = self.value(context, site, flow, &tested)?;
        let absent = value.closures.is_empty()
            && !value.unknown_callable
            && !value.unknown_contents
            && value.types.len() == 1
            && value.types.contains(&None);
        Ok(absent.then_some(present_edge))
    }

    /// A summary's own frames end when it returns. Its caller can only name
    /// storage from the entry, caller and global roots, and storage reachable
    /// from the result, including captured environments. Anything else would
    /// only accumulate per-call names in every caller. Selections are kept
    /// whole so that escaping callback-local selections are still diagnosed.
    fn restrict_exit(
        &mut self,
        context: usize,
        returned: &Value,
        fields: &mut HashMap<Owner, Value>,
        writes: &mut HashMap<Owner, Value>,
    ) -> Result<(), BackendError> {
        let mut by_root = HashMap::<Root, Vec<Owner>>::new();
        for owner in fields.keys().chain(writes.keys()) {
            by_root.entry(owner.root).or_default().push(owner.clone());
        }
        let mut values = vec![returned.clone()];
        let mut roots = by_root
            .keys()
            .copied()
            .filter(|root| matches!(root, Root::Foreign(_)) || !root_is_scoped(*root))
            .collect::<Vec<_>>();
        let mut closures = Vec::new();
        if let Some(entry) = &self.contexts[context].entry {
            values.extend(entry.inputs.iter().cloned());
            values.extend(entry.captures.values().cloned());
            values.extend(entry.fields.values().cloned());
            roots.extend(entry.fields.keys().map(|owner| owner.root));
            closures.extend(entry.closure.clone());
        }
        let mut reached = HashSet::new();
        let mut environments = HashSet::new();
        loop {
            if let Some(value) = values.pop() {
                roots.extend(value.owners.iter().map(|owner| owner.root));
                for closure in &value.closures {
                    roots.push(closure.home.root);
                    closures.push(closure.clone());
                }
            } else if let Some(closure) = closures.pop() {
                if environments.insert((closure.creator, closure.descriptor)) {
                    values.extend(self.environment(
                        context,
                        closure.creator,
                        closure.descriptor,
                    )?);
                }
            } else if let Some(root) = roots.pop() {
                if reached.insert(root) {
                    for owner in by_root.get(&root).into_iter().flatten() {
                        values.extend(fields.get(owner).cloned());
                        values.extend(writes.get(owner).cloned());
                    }
                }
            } else {
                break;
            }
        }
        fields.retain(|owner, _| reached.contains(&owner.root));
        writes.retain(|owner, _| reached.contains(&owner.root));
        Ok(())
    }

    fn select_region(
        &self,
        context: usize,
        flow: &mut Flow,
        request: mir::LocalId,
        region: mir::LocalId,
        checking: bool,
    ) -> Result<(), BackendError> {
        let requests = flow.values[request.0].owners.clone();
        let storage = flow.values[region.0].owners.clone();
        if checking && (requests.is_empty() || storage.is_empty()) {
            return Err(malformed_mir(
                "pending selection has no independently proven request/storage origin",
            ));
        }
        let writable = byte_regions::kind(
            local_in(
                function_in(self.program, self.contexts[context].function)?,
                region,
            )?
            .ty,
        )
        .is_some_and(mir::ByteRegionKind::is_writable);
        for request in requests {
            for storage in &storage {
                let selection = Selection {
                    request: request.root,
                    storage: storage.clone(),
                    writable,
                };
                if checking {
                    selection.validate()?;
                }
                flow.selected.insert(selection);
            }
        }
        Ok(())
    }

    fn statement(
        &mut self,
        context: usize,
        site: Site,
        flow: &mut Flow,
        statement: &mir::Statement,
        checking: bool,
    ) -> Result<(), BackendError> {
        self.calls_in(
            context,
            site,
            flow,
            &collect_statement_class_local_accesses(statement),
            checking,
        )?;
        match statement {
            mir::Statement::AssignLocal { target, value } => {
                if checking && !self.is_region_local(context, *target)? {
                    self.invalidate(flow, &flow.values[target.0], true)?;
                }
                let mut replacement = self.aggregate_value(context, site, flow, value)?;
                if self.purpose == Purpose::Writeback {
                    let function = function_in(self.program, self.contexts[context].function)?;
                    writeback::assign(
                        self.program,
                        function,
                        *target,
                        &flow.values[target.0],
                        &mut replacement,
                        checking,
                    )?;
                }
                self.store_local(context, flow, *target, &mut replacement)?;
                flow.values[target.0] = replacement;
            }
            mir::Statement::AssignLocalGroup { targets, value } => {
                let value = self.aggregate_value(context, site, flow, value)?;
                for target in targets {
                    if checking && !self.is_region_local(context, *target)? {
                        self.invalidate(flow, &flow.values[target.0], true)?;
                    }
                    let mut replacement = value.clone();
                    if self.purpose == Purpose::Writeback {
                        let function = function_in(self.program, self.contexts[context].function)?;
                        writeback::assign(
                            self.program,
                            function,
                            *target,
                            &flow.values[target.0],
                            &mut replacement,
                            checking,
                        )?;
                    }
                    self.store_local(context, flow, *target, &mut replacement)?;
                    flow.values[target.0] = replacement;
                }
            }
            mir::Statement::AssignProperty {
                object,
                property,
                value,
                ..
            } => {
                let region_descriptor = byte_regions::kind(value.ty()).is_some();
                let value = self.aggregate_value(context, site, flow, value)?;
                for owner in flow.values[object.0].owners.clone() {
                    let mut value = value.clone();
                    let selected = owner.project(*property);
                    if checking {
                        self.invalidate(
                            flow,
                            &Value {
                                owners: HashSet::from([selected.clone()]),
                                closures: HashSet::new(),
                                ..Value::default()
                            },
                            true,
                        )?;
                    }
                    if property_in(self.program, property.class, *property)?
                        .borrowed_source
                        .is_none()
                        && !region_descriptor
                    {
                        flow.rehome(&mut value, selected.clone());
                    }
                    flow.fields.insert(selected, value.clone());
                }
            }
            mir::Statement::DropClass { local, .. }
            | mir::Statement::DropCollection { local, .. }
            | mir::Statement::DropFunction { local, .. }
            | mir::Statement::DropMixed { local }
            | mir::Statement::CleanupConstructorPhase { object: local, .. }
                if checking =>
            {
                self.invalidate(flow, &flow.values[local.0], true)?;
            }
            mir::Statement::CollectionAdd {
                collection, value, ..
            }
            | mir::Statement::CollectionSet {
                collection, value, ..
            }
            | mir::Statement::AssignCollectionIndex {
                collection, value, ..
            } => {
                let value = self.aggregate_value(context, site, flow, value)?;
                let owners = flow.values[collection.0].owners.clone();
                self.store_element(flow, &owners, value, Projection::Element(None));
            }
            mir::Statement::CoreCollection {
                collection,
                operation,
            } => {
                use mir::CoreCollectionOperation as O;
                if let O::ValueAt { target, .. }
                | O::Remove { value: target, .. }
                | O::Exchange {
                    previous: target, ..
                } = operation
                {
                    let mut value = self.collection_value(context, flow, *collection)?;
                    self.store_local(context, flow, *target, &mut value)?;
                    flow.values[target.0] = value;
                }
                if let O::KeyAt { target, .. }
                | O::Remove {
                    key: Some(target), ..
                } = operation
                {
                    let mut value =
                        self.collection_projection(context, flow, *collection, Projection::Key)?;
                    self.store_local(context, flow, *target, &mut value)?;
                    flow.values[target.0] = value;
                }
                if let O::Insert { value, .. } | O::Exchange { value, .. } = operation {
                    let owners = flow.values[collection.0].owners.clone();
                    self.store_element(
                        flow,
                        &owners,
                        flow.values[value.0].clone(),
                        Projection::Element(None),
                    );
                    if let O::Insert { key: Some(key), .. } = operation {
                        self.store_element(
                            flow,
                            &owners,
                            flow.values[key.0].clone(),
                            Projection::Key,
                        );
                    }
                }
            }
            mir::Statement::BindPayloadEnumFields {
                source,
                case,
                targets,
                ..
            } => {
                let function = function_in(self.program, self.contexts[context].function)?;
                for (index, target) in targets.iter().enumerate() {
                    let ty = local_in(function, *target)?.ty;
                    let region = byte_regions::kind(ty).is_some();
                    let mut value = Value::default();
                    for owner in &flow.values[source.0].owners {
                        let selected = owner.projected(Projection::Payload(*case, index));
                        if let Some(stored) = flow.field(&selected) {
                            value.join(&stored);
                        } else if flow.values[source.0].unknown_contents {
                            value.mark_unknown_callable(ty);
                            if region {
                                value.owners.insert(Owner::new(Root::Unknown));
                            }
                        }
                        if !region {
                            value.owners.insert(selected);
                        }
                    }
                    self.store_local(context, flow, *target, &mut value)?;
                    flow.values[target.0] = value;
                }
            }
            // Descriptor last use, request.complete and outcome observation are not retirement.
            _ => {}
        }
        Ok(())
    }

    fn store_local(
        &self,
        context: usize,
        flow: &mut Flow,
        target: mir::LocalId,
        value: &mut Value,
    ) -> Result<(), BackendError> {
        let function = function_in(self.program, self.contexts[context].function)?;
        let local = local_in(function, target)?;
        let region = self.is_region_local(context, target)?;
        if function.is_writable_borrowed_place(target) {
            let previous = flow.values[target.0].clone();
            flow.write(&previous, value);
            // Replacing a descriptor writes its home, but does not relocate
            // the byte storage retained by the replacement descriptor.
            if !region && previous.owners.len() == 1 {
                flow.rehome(
                    value,
                    previous
                        .owners
                        .iter()
                        .next()
                        .expect("one storage owner")
                        .clone(),
                );
            }
        } else if !region && (local.owned || !local.ty.has_move_ownership()) {
            flow.rehome(
                value,
                Owner::new(Root::Local(self.contexts[context].scope, Some(target))),
            );
        }
        Ok(())
    }

    fn is_region_local(&self, context: usize, local: mir::LocalId) -> Result<bool, BackendError> {
        let function = function_in(self.program, self.contexts[context].function)?;
        Ok(byte_regions::kind(local_in(function, local)?.ty).is_some())
    }

    fn invalidate(&self, flow: &Flow, value: &Value, exclusive: bool) -> Result<(), BackendError> {
        if value.owners.iter().any(|owner| {
            flow.selected.iter().any(|selection| {
                owner.overlaps(&selection.storage) && (exclusive || selection.writable)
            })
        }) {
            return Err(malformed_mir(
                "selected storage is accessed, replaced or ended before pending retirement",
            ));
        }
        Ok(())
    }

    fn calls_in(
        &mut self,
        context: usize,
        mut site: Site,
        flow: &mut Flow,
        accesses: &ClassLocalAccesses<'_>,
        checking: bool,
    ) -> Result<(), BackendError> {
        let mut storage = accesses.storage_accesses.iter().peekable();
        let mut conditional_entries = Vec::new();
        for (position, access) in accesses.iter().map(Some).chain([None]).enumerate() {
            while let Some((at, _)) = storage.peek() {
                if *at != position {
                    break;
                }
                let (_, access) = storage.next().expect("pending storage access position");
                if checking {
                    match access {
                        StorageAccess::Read(_) | StorageAccess::Independent(_) => {}
                        StorageAccess::Contents(local, writable) => {
                            self.invalidate(flow, &flow.values[local.0], *writable)?;
                        }
                        StorageAccess::ContentsValue(value) => {
                            let value = self.value(
                                context,
                                value_site(site, accesses, value)?,
                                flow,
                                value,
                            )?;
                            self.invalidate(flow, &value, false)?;
                        }
                        StorageAccess::Acquire(source, writable) => {
                            let value = self.value(
                                context,
                                value_site(site, accesses, source)?,
                                flow,
                                source,
                            )?;
                            self.invalidate(flow, &value, *writable)?;
                        }
                        StorageAccess::Transfer(local) => {
                            if !self.is_region_local(context, *local)? {
                                self.invalidate(flow, &flow.values[local.0], true)?;
                            }
                        }
                    }
                }
            }
            let Some(access) = access else {
                continue;
            };
            match access {
                ClassLocalAccess::BeginConditional => conditional_entries.push(flow.clone()),
                ClassLocalAccess::EndConditional => {
                    flow.join(
                        conditional_entries.pop().ok_or_else(|| {
                            malformed_mir("conditional access markers are unbalanced")
                        })?,
                        false,
                    );
                }
                ClassLocalAccess::Call(
                    CallTarget::Direct {
                        function,
                        parameter_offset,
                    },
                    args,
                ) => {
                    let mut inputs = Vec::new();
                    if parameter_offset == 1 {
                        // Constructors lend newly created storage, not the
                        // registered adapter. Null-safe methods instead lend
                        // the actual receiver selected by the MIR operation.
                        let receiver = accesses
                            .method_receivers
                            .iter()
                            .find_map(|(at, _, receiver)| (*at == position).then_some(*receiver));
                        inputs.push(if let Some(receiver) = receiver {
                            let receiver = mir::Rvalue::NullableClass(receiver.clone());
                            let preceding = value_call_count(&receiver)
                                + args.iter().map(value_call_count).sum::<usize>();
                            let receiver_site = Site {
                                call: site.call.checked_sub(preceding).ok_or_else(|| {
                                    malformed_mir("pending receiver call order is inconsistent")
                                })?,
                                ..site
                            };
                            self.value(context, receiver_site, flow, &receiver)?
                        } else {
                            let span = accesses
                                .immediate_calls
                                .iter()
                                .find_map(|(at, span)| (*at == position).then_some(*span));
                            let own = self.contexts[context].scope;
                            let root = if let Some(span) = span {
                                Root::Construction(own, site, span)
                            } else {
                                let body =
                                    function_in(self.program, self.contexts[context].function)?;
                                let block = &body.blocks[site.block.0];
                                let mir::Terminator::CheckedConstruct { result, .. } =
                                    block.terminator
                                else {
                                    return Err(malformed_mir(
                                        "constructor has no construction identity",
                                    ));
                                };
                                if site.statement != block.statements.len() {
                                    return Err(malformed_mir(
                                        "checked constructor has no result storage",
                                    ));
                                }
                                Root::Local(own, Some(result))
                            };
                            Value::owner(root)
                        });
                    }
                    for arg in args {
                        inputs.push(self.aggregate_value(
                            context,
                            value_site(site, accesses, arg)?,
                            flow,
                            arg,
                        )?);
                    }
                    let receiver = accesses
                        .method_receivers
                        .iter()
                        .find_map(|(at, _, receiver)| {
                            (*at == position).then_some(receiver.static_receiver_class())
                        })
                        .or_else(|| {
                            (parameter_offset == 0)
                                .then(|| retained::receiver_class(args))
                                .flatten()
                        });
                    for target in possible_callees(self.program, function, receiver)? {
                        let id = self.context(context, site, target.id, None, &inputs, flow)?;
                        self.apply(id, flow, checking)?;
                    }
                    site.call += 1;
                }
                ClassLocalAccess::Transfer(local) if checking => {
                    self.invalidate(flow, &flow.values[local.0], true)?
                }
                _ => {}
            }
        }
        if !conditional_entries.is_empty() {
            return Err(malformed_mir("conditional access markers are unbalanced"));
        }
        Ok(())
    }

    fn apply(&mut self, call: Call, flow: &mut Flow, checking: bool) -> Result<(), BackendError> {
        match call {
            Call::Context(context) => {
                let context = &self.contexts[context];
                apply_exit(
                    self.purpose,
                    &context.outgoing,
                    &context.outgoing_fields,
                    &context.outgoing_values,
                    flow,
                    checking,
                )
            }
            Call::Record(record) => {
                let exit = self.instantiated(record)?;
                apply_exit(
                    self.purpose,
                    &exit.outgoing,
                    &exit.outgoing_fields,
                    &exit.outgoing_values,
                    flow,
                    checking,
                )
            }
        }
    }

    fn returned(&mut self, call: Call) -> Result<Value, BackendError> {
        Ok(match call {
            Call::Context(context) => self.contexts[context].returned.clone(),
            Call::Record(record) => self.instantiated(record)?.returned.clone(),
        })
    }

    fn may_return(&self, call: Call) -> bool {
        match call {
            Call::Context(context) => self.contexts[context].normal_return,
            Call::Record(record) => self.records[record]
                .summary
                .is_some_and(|summary| self.contexts[summary].normal_return),
        }
    }

    fn context(
        &mut self,
        caller: usize,
        site: Site,
        function: mir::FunctionId,
        closure: Option<Closure>,
        inputs: &[Value],
        flow: &Flow,
    ) -> Result<Call, BackendError> {
        let key = (caller, site, function, closure.clone());
        // A recursive cycle keeps one context per call path, folded into the
        // nearest same-function ancestor. That ancestor is always inside the
        // cycle's own component, so a component boundary may summarize.
        if self.components[self.contexts[caller].function.0] != self.components[function.0] {
            return self.record(key, inputs, flow);
        }
        let recursive = self.recursive_context(caller, function);
        let id = if let Some(Call::Context(id)) = self.calls.get(&key) {
            *id
        } else {
            let id = recursive.unwrap_or_else(|| {
                let id = self.contexts.len();
                let scope = self.scopes.intern(Scope::Within(
                    self.contexts[caller].scope,
                    Step::Call(site, function, closure.clone()),
                ));
                let root = self.contexts[caller].root;
                let inputs =
                    vec![Value::default(); self.program.functions[function.0].params.len()];
                self.contexts.push(Context::new(
                    function,
                    Some(caller),
                    Kind::Child,
                    scope,
                    root,
                    inputs,
                ));
                self.changed = true;
                id
            });
            self.calls.insert(key, Call::Context(id));
            id
        };
        self.dependents.entry(id).or_default().insert(caller);
        let offset = usize::from(closure.is_some());
        let mut changed = false;
        for (index, value) in inputs.iter().enumerate() {
            let input = self.contexts[id]
                .inputs
                .get_mut(index + offset)
                .ok_or_else(|| {
                    malformed_mir("pending proof call argument does not match its callee")
                })?;
            changed |= input.join_on_cycle(value, recursive.is_some());
        }
        let before = self.contexts[id].selected.len();
        self.contexts[id]
            .selected
            .extend(flow.selected.iter().cloned());
        changed |= self.contexts[id].selected.len() != before;
        changed |= join_storage(
            &mut self.contexts[id].fields,
            &flow.fields,
            recursive.is_some(),
        );
        if let Some(closure) = closure {
            changed |= self.contexts[id].inputs[0].join_on_cycle(
                &Value {
                    owners: HashSet::from([closure.home.clone()]),
                    closures: HashSet::from([closure.clone()]),
                    ..Value::default()
                },
                recursive.is_some(),
            );
            self.bind_closure(id, &closure, recursive.is_some(), caller)?;
        }
        self.contexts[id].dirty |= changed;
        self.changed |= changed;
        Ok(Call::Context(id))
    }

    /// Accumulates a component-crossing call's entry, as a path context would,
    /// and binds it to the summary of that entry's canonical form.
    fn record(
        &mut self,
        key: (usize, Site, mir::FunctionId, Option<Closure>),
        inputs: &[Value],
        flow: &Flow,
    ) -> Result<Call, BackendError> {
        let (caller, site, function, closure) = key.clone();
        let id = if let Some(Call::Record(id)) = self.calls.get(&key) {
            *id
        } else {
            let frame = self.scopes.intern(Scope::Within(
                self.contexts[caller].scope,
                Step::Call(site, function, closure.clone()),
            ));
            let parameters = function_in(self.program, function)?.params.len();
            let id = self.records.len();
            self.records.push(Record {
                caller,
                function,
                closure: closure.clone(),
                frame,
                inputs: vec![Value::default(); parameters],
                captures: HashMap::new(),
                fields: HashMap::new(),
                selected: HashSet::new(),
                summary: None,
                sigma: Vec::new(),
                sigma_roots: Vec::new(),
                rebased: HashMap::new(),
                exit: None,
            });
            self.calls.insert(key, Call::Record(id));
            id
        };
        let offset = usize::from(closure.is_some());
        let record = &mut self.records[id];
        for (index, value) in inputs.iter().enumerate() {
            record
                .inputs
                .get_mut(index + offset)
                .ok_or_else(|| {
                    malformed_mir("pending proof call argument does not match its callee")
                })?
                .join(value);
        }
        record.selected.extend(flow.selected.iter().cloned());
        join_storage(&mut record.fields, &flow.fields, false);
        if let Some(closure) = &closure {
            record
                .inputs
                .first_mut()
                .ok_or_else(|| malformed_mir("pending callback has no environment input"))?
                .join(&Value {
                    owners: HashSet::from([closure.home.clone()]),
                    closures: HashSet::from([closure.clone()]),
                    ..Value::default()
                });
            let mut fields = std::mem::take(&mut record.fields);
            let mut captures = std::mem::take(&mut record.captures);
            let frame = record.frame;
            let bound = self.bind_captures(
                caller,
                function,
                closure,
                frame,
                false,
                &mut fields,
                &mut captures,
            );
            self.records[id].fields = fields;
            self.records[id].captures = captures;
            bound?;
        }
        self.rebind(id)?;
        Ok(Call::Record(id))
    }

    fn rebind(&mut self, id: usize) -> Result<(), BackendError> {
        let (entry, sigma, sigma_roots) = self.canonicalize(id)?;
        let key = (entry.function, entry_fingerprint(&entry));
        let existing = self.summaries.get(&key).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|candidate| self.contexts[*candidate].entry.as_ref() == Some(&entry))
        });
        let summary = if let Some(summary) = existing {
            summary
        } else {
            let summary = self.contexts.len();
            let mut context = Context::new(
                entry.function,
                None,
                Kind::Summary,
                OWN,
                summary,
                entry.inputs.clone(),
            );
            context.captures.clone_from(&entry.captures);
            context.fields.clone_from(&entry.fields);
            context.selected.clone_from(&entry.selected);
            context.entry = Some(entry);
            self.contexts.push(context);
            self.summaries.entry(key).or_default().push(summary);
            self.changed = true;
            summary
        };
        let record = &mut self.records[id];
        if record.summary != Some(summary)
            || record.sigma != sigma
            || record.sigma_roots != sigma_roots
        {
            let rebound = record.summary.is_some();
            record.summary = Some(summary);
            record.sigma = sigma;
            record.sigma_roots = sigma_roots;
            record.rebased.clear();
            record.exit = None;
            if rebound {
                // Environments resolved through this call now live elsewhere.
                for reader in self.record_readers.get(&id).into_iter().flatten() {
                    self.contexts[*reader].dirty = true;
                }
                self.changed = true;
            }
        }
        let caller = self.records[id].caller;
        self.dependents.entry(summary).or_default().insert(caller);
        Ok(())
    }

    /// The callee-naming entry of a record, with the caller scope and storage
    /// behind each foreign scope and root. Storage inside the callee's own
    /// frame flowed back from an earlier call and returns to the callee's own
    /// naming. Every other caller root and closure creator is numbered in
    /// discovery order, so entries that differ only in caller naming match.
    #[allow(clippy::type_complexity)]
    fn canonicalize(&mut self, id: usize) -> Result<(Entry, Vec<usize>, Vec<Root>), BackendError> {
        let caller = self.records[id].caller;
        let function = self.records[id].function;
        let closure = self.records[id].closure.clone();
        let inputs = self.records[id].inputs.clone();
        let captures = self.records[id].captures.clone();
        let fields = self.records[id].fields.clone();
        let selected = self.records[id].selected.clone();
        let mut state = Canonical {
            frame: self.records[id].frame,
            assigned: HashMap::new(),
            foreign: Vec::new(),
            roots: HashMap::new(),
            foreign_roots: Vec::new(),
            reached: HashSet::new(),
            frontier: std::collections::VecDeque::new(),
            closures: HashSet::new(),
            pending: std::collections::VecDeque::new(),
        };
        let mut by_root = HashMap::<Root, Vec<Owner>>::new();
        for owner in fields.keys() {
            by_root.entry(owner.root).or_default().push(owner.clone());
        }
        for owners in by_root.values_mut() {
            owners.sort_unstable_by_key(owner_shape);
        }
        for input in &inputs {
            self.discover(&mut state, input)?;
        }
        let mut locals = captures.keys().copied().collect::<Vec<_>>();
        locals.sort_unstable();
        for local in &locals {
            self.discover(&mut state, &captures[local])?;
        }
        // Statics and other global roots are visible to every callee.
        let mut globals = by_root
            .keys()
            .copied()
            .filter(|root| !root_is_scoped(*root))
            .collect::<Vec<_>>();
        globals.sort_unstable_by_key(|root| owner_shape(&Owner::new(*root)));
        for root in globals {
            if state.reached.insert(root) {
                state.frontier.push_back(root);
            }
        }
        let mut environments = Vec::new();
        loop {
            if let Some(closure) = state.pending.pop_front() {
                let values = self.environment(caller, closure.creator, closure.descriptor)?;
                for value in &values {
                    self.discover(&mut state, value)?;
                }
                environments.push((closure, values));
            } else if let Some(root) = state.frontier.pop_front() {
                for owner in by_root.get(&root).into_iter().flatten() {
                    self.discover(&mut state, &fields[owner])?;
                }
            } else {
                break;
            }
        }
        let failure = || malformed_mir("call entry names storage outside its caller");
        let mut entry = Entry {
            function,
            closure: None,
            inputs: Vec::new(),
            captures: HashMap::new(),
            fields: HashMap::new(),
            selected: HashSet::new(),
            environments: HashMap::new(),
        };
        entry.closure = match &closure {
            Some(closure) => Some(
                map_closure(
                    closure,
                    &mut Canonicalize {
                        proof: self,
                        state: &mut state,
                    },
                )
                .ok_or_else(failure)?,
            ),
            None => None,
        };
        for input in &inputs {
            let value = map_value(
                input,
                &mut Canonicalize {
                    proof: self,
                    state: &mut state,
                },
            )
            .ok_or_else(failure)?;
            entry.inputs.push(value);
        }
        for local in locals {
            let value = map_value(
                &captures[&local],
                &mut Canonicalize {
                    proof: self,
                    state: &mut state,
                },
            )
            .ok_or_else(failure)?;
            entry.captures.insert(local, value);
        }
        for (owner, value) in &fields {
            if !state.reached.contains(&owner.root) {
                continue;
            }
            let mut rename = Canonicalize {
                proof: self,
                state: &mut state,
            };
            let owner = map_owner(owner, &mut rename).ok_or_else(failure)?;
            let value = map_value(value, &mut rename).ok_or_else(failure)?;
            entry.fields.insert(owner, value);
        }
        for selection in &selected {
            if state.reached.contains(&selection.storage.root) {
                let selection = map_selection(
                    selection,
                    &mut Canonicalize {
                        proof: self,
                        state: &mut state,
                    },
                )
                .ok_or_else(failure)?;
                entry.selected.insert(selection);
            }
        }
        for (closure, values) in environments {
            let creator = self
                .canonical_scope(&mut state, closure.creator)
                .ok_or_else(failure)?;
            // A summary resolves environments created inside its own frame.
            if !matches!(self.scopes.get(creator), Scope::Foreign(_)) {
                continue;
            }
            let mut rename = Canonicalize {
                proof: self,
                state: &mut state,
            };
            let values = values
                .iter()
                .map(|value| map_value(value, &mut rename))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(failure)?;
            entry
                .environments
                .insert((creator, closure.descriptor), values);
        }
        Ok((entry, state.foreign, state.foreign_roots))
    }

    fn discover(&mut self, state: &mut Canonical, value: &Value) -> Result<(), BackendError> {
        let mut owners = value.owners.iter().collect::<Vec<_>>();
        owners.sort_unstable_by_key(|owner| owner_shape(owner));
        for owner in owners {
            self.reach(state, owner.root)?;
        }
        let mut closures = value.closures.iter().collect::<Vec<_>>();
        closures.sort_unstable_by_key(|closure| closure_shape(closure));
        for closure in closures {
            self.reach(state, closure.home.root)?;
            self.canonical_scope(state, closure.creator)
                .ok_or_else(|| malformed_mir("callback creator has no caller scope"))?;
            if state.closures.insert((closure.creator, closure.descriptor)) {
                state.pending.push_back(closure.clone());
            }
        }
        Ok(())
    }

    fn reach(&mut self, state: &mut Canonical, root: Root) -> Result<(), BackendError> {
        self.canonical_root(state, root)
            .ok_or_else(|| malformed_mir("call entry storage has no caller scope"))?;
        if state.reached.insert(root) {
            state.frontier.push_back(root);
        }
        Ok(())
    }

    fn canonical_root(&mut self, state: &mut Canonical, root: Root) -> Option<Root> {
        if !root_is_scoped(root) {
            return Some(root);
        }
        if let Some(mapped) = state.roots.get(&root) {
            return Some(*mapped);
        }
        let mapped = match root_scope(root) {
            Some(scope) if self.scopes.under(scope, state.frame) => rename_root_scope(
                root,
                &mut Canonicalize {
                    proof: self,
                    state: &mut *state,
                },
            )?,
            _ => {
                state.foreign_roots.push(root);
                Root::Foreign(state.foreign_roots.len() - 1)
            }
        };
        state.roots.insert(root, mapped);
        Some(mapped)
    }

    fn canonical_scope(&mut self, state: &mut Canonical, scope: usize) -> Option<usize> {
        if let Some(mapped) = state.assigned.get(&scope) {
            return Some(*mapped);
        }
        let mapped = if scope == state.frame {
            OWN
        } else {
            match self.scopes.get(scope).clone() {
                Scope::Within(parent, step) if self.scopes.under(parent, state.frame) => {
                    let parent = self.canonical_scope(state, parent)?;
                    let step = map_step(
                        &step,
                        &mut Canonicalize {
                            proof: self,
                            state: &mut *state,
                        },
                    )?;
                    self.scopes.intern(Scope::Within(parent, step))
                }
                _ => {
                    state.foreign.push(scope);
                    self.scopes.intern(Scope::Foreign(state.foreign.len() - 1))
                }
            }
        };
        state.assigned.insert(scope, mapped);
        Some(mapped)
    }

    /// Maps a summary scope into the naming of one record's caller. Storage
    /// the callee created lives below the record's frame, as a path context's
    /// storage would; caller storage returns to the caller's own scopes.
    fn rebase_scope(&mut self, record: usize, scope: usize) -> Option<usize> {
        if let Some(mapped) = self.records[record].rebased.get(&scope) {
            return Some(*mapped);
        }
        let mapped = match self.scopes.get(scope).clone() {
            Scope::Own => self.records[record].frame,
            Scope::Foreign(index) => *self.records[record].sigma.get(index)?,
            Scope::Within(parent, step) => {
                let parent = self.rebase_scope(record, parent)?;
                let step = map_step(
                    &step,
                    &mut Rebase {
                        proof: self,
                        record,
                    },
                )?;
                self.scopes.intern(Scope::Within(parent, step))
            }
        };
        self.records[record].rebased.insert(scope, mapped);
        Some(mapped)
    }

    /// The inverse of `rebase_scope` for scopes the summary can name.
    fn unrebase_scope(&mut self, record: usize, scope: usize) -> Option<usize> {
        let frame = self.records[record].frame;
        if scope == frame {
            return Some(OWN);
        }
        match self.scopes.get(scope).clone() {
            Scope::Within(parent, step) if self.scopes.under(parent, frame) => {
                let parent = self.unrebase_scope(record, parent)?;
                let step = map_step(
                    &step,
                    &mut Unrebase {
                        proof: self,
                        record,
                    },
                )?;
                Some(self.scopes.intern(Scope::Within(parent, step)))
            }
            _ => self.records[record]
                .sigma
                .iter()
                .position(|caller| *caller == scope)
                .map(|index| self.scopes.intern(Scope::Foreign(index))),
        }
    }

    fn unrebase_root(&mut self, record: usize, root: Root) -> Option<Root> {
        if !root_is_scoped(root) {
            return Some(root);
        }
        let frame = self.records[record].frame;
        if root_scope(root).is_some_and(|scope| self.scopes.under(scope, frame)) {
            return rename_root_scope(
                root,
                &mut Unrebase {
                    proof: self,
                    record,
                },
            );
        }
        self.records[record]
            .sigma_roots
            .iter()
            .position(|caller| *caller == root)
            .map(Root::Foreign)
    }

    fn instantiated(&mut self, record: usize) -> Result<std::rc::Rc<Exit>, BackendError> {
        let summary = self.records[record]
            .summary
            .ok_or_else(|| malformed_mir("call record has no summary"))?;
        let version = self.contexts[summary].version;
        if let Some((cached, exit)) = &self.records[record].exit {
            if *cached == version {
                return Ok(exit.clone());
            }
        }
        let context = &self.contexts[summary];
        let returned = context.returned.clone();
        let outgoing = context.outgoing.clone();
        let fields = context.outgoing_fields.clone();
        let values = context.outgoing_values.clone();
        let mut rename = Rebase {
            proof: self,
            record,
        };
        let exit = map_value(&returned, &mut rename)
            .and_then(|returned| {
                Some(Exit {
                    returned,
                    outgoing: outgoing
                        .iter()
                        .map(|selection| map_selection(selection, &mut rename))
                        .collect::<Option<_>>()?,
                    outgoing_fields: map_storage(&fields, &mut rename)?,
                    outgoing_values: map_storage(&values, &mut rename)?,
                })
            })
            .ok_or_else(|| malformed_mir("call summary names storage outside its entry"))?;
        let exit = std::rc::Rc::new(exit);
        self.records[record].exit = Some((version, exit.clone()));
        Ok(exit)
    }

    /// The context whose frame `scope` names inside `root`'s tree, with the
    /// records crossed on the way there, outermost first.
    fn frame(
        &mut self,
        root: usize,
        scope: usize,
    ) -> Result<Option<(usize, Vec<usize>)>, BackendError> {
        let (parent, step) = match self.scopes.get(scope).clone() {
            Scope::Own => return Ok(Some((root, Vec::new()))),
            Scope::Foreign(_) => return Ok(None),
            Scope::Within(parent, step) => (parent, step),
        };
        let Some((context, mut chain)) = self.frame(root, parent)? else {
            return Ok(None);
        };
        let mut step = Some(step);
        for record in &chain {
            step = step.and_then(|step| {
                map_step(
                    &step,
                    &mut Unrebase {
                        proof: self,
                        record: *record,
                    },
                )
            });
        }
        Ok(match step {
            Some(Step::Call(site, function, closure)) => {
                match self.calls.get(&(context, site, function, closure)).copied() {
                    Some(Call::Context(child)) => Some((child, chain)),
                    Some(Call::Record(record)) => {
                        chain.push(record);
                        self.records[record].summary.map(|summary| (summary, chain))
                    }
                    None => None,
                }
            }
            Some(Step::Definition(descriptor)) => self
                .definitions
                .get(&(context, descriptor))
                .and_then(|definition| definition.context)
                .map(|definition| (definition, chain)),
            None => None,
        })
    }

    /// The captured values of a closure, in `reader`'s tree naming. Readers are
    /// revisited when the environment changes or a crossed call re-binds.
    fn environment(
        &mut self,
        reader: usize,
        creator: usize,
        descriptor: mir::ClosureDescriptorId,
    ) -> Result<Vec<Value>, BackendError> {
        let root = self.contexts[reader].root;
        if matches!(self.scopes.get(creator), Scope::Foreign(_)) {
            return Ok(self.contexts[root]
                .entry
                .as_ref()
                .and_then(|entry| entry.environments.get(&(creator, descriptor)))
                .cloned()
                .unwrap_or_default());
        }
        let Some((context, chain)) = self.frame(root, creator)? else {
            return Ok(Vec::new());
        };
        self.environment_readers
            .entry((context, descriptor))
            .or_default()
            .insert(reader);
        for record in &chain {
            self.record_readers
                .entry(*record)
                .or_default()
                .insert(reader);
        }
        let mut values = self
            .environments
            .get(&(context, descriptor))
            .cloned()
            .unwrap_or_default();
        for record in chain.into_iter().rev() {
            let mut rename = Rebase {
                proof: self,
                record,
            };
            values = values
                .iter()
                .map(|value| map_value(value, &mut rename))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    malformed_mir("callback environment names storage outside its call")
                })?;
        }
        Ok(values)
    }

    fn recursive_context(&self, caller: usize, function: mir::FunctionId) -> Option<usize> {
        let mut ancestor = Some(caller);
        while let Some(id) = ancestor {
            if self.contexts[id].function == function {
                return Some(id);
            }
            ancestor = self.contexts[id].parent;
        }
        None
    }

    fn bind_closure(
        &mut self,
        id: usize,
        closure: &Closure,
        cyclic: bool,
        reader: usize,
    ) -> Result<(), BackendError> {
        let mut fields = std::mem::take(&mut self.contexts[id].fields);
        let mut captures = std::mem::take(&mut self.contexts[id].captures);
        let function = self.contexts[id].function;
        let frame = self.contexts[id].scope;
        let bound = self.bind_captures(
            reader,
            function,
            closure,
            frame,
            cyclic,
            &mut fields,
            &mut captures,
        );
        self.contexts[id].fields = fields;
        self.contexts[id].captures = captures;
        let changed = bound?;
        self.contexts[id].dirty |= changed;
        self.changed |= changed;
        Ok(())
    }

    /// Binds a callback's captured values for the callee whose own frame is
    /// `frame`, relocating owned captures into the callee's environment home.
    #[allow(clippy::too_many_arguments)]
    fn bind_captures(
        &mut self,
        reader: usize,
        function: mir::FunctionId,
        closure: &Closure,
        frame: usize,
        cyclic: bool,
        fields: &mut HashMap<Owner, Value>,
        captures: &mut HashMap<mir::LocalId, Value>,
    ) -> Result<bool, BackendError> {
        let descriptor = closure_descriptor_in(self.program, closure.descriptor)?;
        let function = function_in(self.program, function)?;
        let metadata = function
            .closure
            .as_ref()
            .ok_or_else(|| malformed_mir("pending callback lacks validated capture metadata"))?;
        let captured = self.environment(reader, closure.creator, closure.descriptor)?;
        let mut changed = false;
        if let Some(layout) = descriptor.environment_layout {
            let layout = closure_environment_layout_in(self.program, layout)?;
            for (field, local) in &metadata.capture_locals {
                let field = layout
                    .fields
                    .iter()
                    .find(|value| value.id == *field)
                    .ok_or_else(|| malformed_mir("pending capture field is missing"))?;
                let mut value = captured
                    .get(field.logical_index)
                    .cloned()
                    .unwrap_or_default();
                if field.storage == mir::ClosureEnvironmentStorage::Owned
                    && byte_regions::kind(field.ty).is_none()
                {
                    changed |= Flow::rehome_fields(
                        fields,
                        &mut value,
                        if descriptor.invocation_mode == mir::FunctionInvocationMode::Once {
                            Owner::new(Root::Local(frame, Some(*local)))
                        } else {
                            closure.home.capture(field.id)
                        },
                    );
                    if self.purpose == Purpose::Writeback {
                        value.homes = HashSet::from([Some(field.ty)]);
                    }
                }
                changed |= captures
                    .entry(*local)
                    .or_default()
                    .join_on_cycle(&value, cyclic);
            }
        }
        Ok(changed)
    }

    fn refresh_environment(
        &mut self,
        creator: usize,
        descriptor: mir::ClosureDescriptorId,
    ) -> Result<(), BackendError> {
        self.changed = true;
        for reader in self
            .environment_readers
            .get(&(creator, descriptor))
            .into_iter()
            .flatten()
        {
            self.contexts[*reader].dirty = true;
        }
        // Definition contexts are not callers, even after invocation is known.
        let definition = self
            .definitions
            .get(&(creator, descriptor))
            .and_then(|definition| {
                definition
                    .context
                    .map(|id| (id, definition.closure.clone(), definition.fields.clone()))
            });
        if let Some((id, closure, fields)) = definition {
            let changed = join_fields(&mut self.contexts[id].fields, &fields);
            self.contexts[id].dirty |= changed;
            self.changed |= changed;
            self.bind_closure(id, &closure, false, id)?;
        }
        Ok(())
    }

    fn indirect(
        &mut self,
        context: usize,
        site: Site,
        flow: &mut Flow,
        callee: &mir::IndirectCallee,
        args: &[mir::Rvalue],
        checking: bool,
    ) -> Result<Value, BackendError> {
        let mut accesses = ClassLocalAccesses::default();
        if let mir::IndirectCallee::Closure(expression) = callee {
            collect_function_class_local_accesses(expression, &mut accesses);
        }
        collect_rvalue_args_class_local_accesses(args, &mut accesses);
        let mut result = Value::default();
        let mut inputs = Vec::new();
        for arg in args {
            inputs.push(self.value(context, value_site(site, &accesses, arg)?, flow, arg)?);
        }
        let call_site = Site {
            call: accesses.direct_calls,
            ..site
        };
        match callee {
            mir::IndirectCallee::InterfaceMethod {
                interface, slot, ..
            } => {
                let targets = self
                    .program
                    .interface_vtables
                    .iter()
                    .filter(|table| table.interface == *interface)
                    .map(|table| table.methods[*slot])
                    .collect::<Vec<_>>();
                for target in targets {
                    let call = self.context(context, call_site, target, None, &inputs, flow)?;
                    result.join(&self.returned(call)?);
                    self.apply(call, flow, checking)?;
                }
            }
            mir::IndirectCallee::Closure(expression) => {
                let value = self.value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::Function(expression.clone()),
                )?;
                // Bottom during convergence is not an arbitrary callback.
                // Otherwise its universal writes permanently poison known
                // bodies discovered on a later iteration.
                let unknown = value.unknown_callable;
                if checking {
                    let origin = self
                        .callback_origins
                        .entry((self.contexts[context].function, site.block))
                        .or_default();
                    origin.unknown |= unknown;
                    for closure in &value.closures {
                        origin.targets.insert(
                            closure_descriptor_in(self.program, closure.descriptor)?.entry_function,
                        );
                    }
                }
                if checking && !unknown && value.closures.is_empty() {
                    return Err(malformed_mir(
                        "callback has no independently established executable origin",
                    ));
                }
                if self.purpose == Purpose::Writeback && unknown {
                    let signature = function_type_in(self.program, expression.function_type())?;
                    if checking {
                        writeback::unknown_call(self.program, signature, &inputs)?;
                    }
                    for (parameter, input) in signature.parameters.iter().zip(&inputs) {
                        if parameter.mode == mir::FunctionParameterMode::Writable {
                            let mut replacement = Value {
                                types: HashSet::from([Some(parameter.ty)]),
                                ..Value::default()
                            };
                            replacement.mark_unknown_callable(parameter.ty);
                            flow.write(input, &replacement);
                        }
                    }
                    if let mir::ReturnType::Value(ty) = signature.return_type {
                        result.types.insert(Some(ty));
                        result.homes.insert(None);
                        result.mark_unknown_callable(ty);
                    }
                }
                if self.purpose == Purpose::Pending && checking && unknown {
                    return Err(malformed_mir(
                        "pending callback has no independently established body and capture origins",
                    ));
                }
                for closure in value.closures {
                    let target =
                        closure_descriptor_in(self.program, closure.descriptor)?.entry_function;
                    let call =
                        self.context(context, call_site, target, Some(closure), &inputs, flow)?;
                    result.join(&self.returned(call)?);
                    self.apply(call, flow, checking)?;
                }
            }
        }
        Ok(result)
    }

    fn direct_call_may_return(
        &self,
        context: usize,
        site: Site,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
    ) -> Result<bool, BackendError> {
        let calls = self.direct_contexts(context, site, function, args)?;
        Ok(calls.is_empty()
            || calls
                .iter()
                .any(|call| call.is_none_or(|call| self.may_return(call))))
    }

    fn direct_result(
        &mut self,
        context: usize,
        site: Site,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
        _flow: &Flow,
    ) -> Result<Value, BackendError> {
        let mut result = Value::default();
        for call in self
            .direct_contexts(context, site, function, args)?
            .into_iter()
            .flatten()
        {
            result.join(&self.returned(call)?);
        }
        Ok(result)
    }

    fn direct_contexts(
        &self,
        context: usize,
        mut site: Site,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
    ) -> Result<Vec<Option<Call>>, BackendError> {
        // A direct call executes after all calls in its argument subtrees.
        site.call += args.iter().map(value_call_count).sum::<usize>();
        let mut contexts = Vec::new();
        for target in possible_callees(self.program, function, retained::receiver_class(args))? {
            // Return provenance belongs to the actual executable call site.
            // Do not merge factory calls by callee name or invent a second call.
            contexts.push(self.calls.get(&(context, site, target.id, None)).copied());
        }
        Ok(contexts)
    }

    fn value(
        &mut self,
        context: usize,
        site: Site,
        flow: &Flow,
        value: &mir::Rvalue,
    ) -> Result<Value, BackendError> {
        let mut result = self.value_origins(context, site, flow, value)?;
        if self.purpose == Purpose::Writeback {
            writeback::complete_value(self.program, value, &mut result)?;
        }
        Ok(result)
    }

    fn aggregate_value(
        &mut self,
        context: usize,
        site: Site,
        flow: &mut Flow,
        value: &mir::Rvalue,
    ) -> Result<Value, BackendError> {
        match value {
            mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(value)) => {
                return self.aggregate_value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::PayloadEnum(value.clone()),
                );
            }
            mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
                return self.aggregate_value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::Class((**value).clone()),
                );
            }
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
                return self.aggregate_value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::NullableClass((**value).clone()),
                );
            }
            _ => {}
        }
        let result = self.value(context, site, flow, value)?;
        match value {
            mir::Rvalue::Collection(mir::CollectionExpression::Literal { entries, .. }) => {
                let mut site = site;
                self.store_element(
                    flow,
                    &result.owners,
                    Value::default(),
                    Projection::Element(None),
                );
                for (position, entry) in entries.iter().enumerate() {
                    let key = entry
                        .key
                        .as_ref()
                        .map(literal_collection_key)
                        .unwrap_or_else(|| {
                            Some(crate::ownership::cleanup::CollectionKey::Integer(
                                position as i128,
                            ))
                        });
                    if let Some(key) = &entry.key {
                        let stored = self.aggregate_value(context, site, flow, key)?;
                        self.store_element(flow, &result.owners, stored, Projection::Key);
                        site.call += value_call_count(key);
                    }
                    let stored = self.aggregate_value(context, site, flow, &entry.value)?;
                    self.store_element(flow, &result.owners, stored, Projection::Element(key));
                    site.call += value_call_count(&entry.value);
                }
            }
            mir::Rvalue::Collection(mir::CollectionExpression::Fill { value, .. }) => {
                let stored = self.aggregate_value(context, site, flow, value)?;
                self.store_element(flow, &result.owners, stored, Projection::Element(None));
            }
            mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(
                value,
            )) => {
                self.aggregate_value(context, site, flow, &mir::Rvalue::Collection(value.clone()))?;
            }
            mir::Rvalue::Class(mir::ClassExpression::New { properties, .. }) => {
                let mut site = site;
                for property in properties {
                    if let mir::PropertyValueSource::Expression(value) = &property.source {
                        let stored = self.aggregate_value(context, site, flow, value)?;
                        for owner in &result.owners {
                            let selected = owner.project(property.property);
                            let relocate = !property_in(
                                self.program,
                                property.property.class,
                                property.property,
                            )?
                            .borrowed_source
                            .is_some()
                                && byte_regions::kind(value.ty()).is_none();
                            flow.store_projected(selected, &stored, relocate);
                        }
                        site.call += value_call_count(value);
                    }
                }
            }
            mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Construct {
                case,
                fields,
                ..
            }) => {
                let mut site = site;
                for (index, field) in fields.iter().enumerate() {
                    let stored = self.aggregate_value(context, site, flow, field)?;
                    let projection = Projection::Payload(*case, index);
                    if byte_regions::kind(field.ty()).is_some() {
                        for owner in &result.owners {
                            flow.store_projected(
                                owner.projected(projection.clone()),
                                &stored,
                                false,
                            );
                        }
                    } else {
                        self.store_element(flow, &result.owners, stored, projection);
                    }
                    site.call += value_call_count(field);
                }
            }
            _ => {}
        }
        Ok(result)
    }

    fn store_element(
        &self,
        flow: &mut Flow,
        owners: &HashSet<Owner>,
        value: Value,
        projection: Projection,
    ) {
        for owner in owners {
            flow.store_projected(owner.projected(projection.clone()), &value, true);
        }
    }

    fn value_origins(
        &mut self,
        context: usize,
        site: Site,
        flow: &Flow,
        value: &mir::Rvalue,
    ) -> Result<Value, BackendError> {
        match value {
            mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
                return self.value(context, site, flow, &mir::Rvalue::Class((**value).clone()));
            }
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
                return self.value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::NullableClass((**value).clone()),
                );
            }
            _ => {}
        }
        if let Some(local) = value.direct_place_local() {
            return Ok(flow.values[local.0].clone());
        }
        if let Some((collection, index)) = retained::collection_element_place(value) {
            return self.collection_projection(
                context,
                flow,
                collection,
                Projection::Element(literal_collection_key(index)),
            );
        }
        if let Some(crate::native_closure_abi::BorrowHomePlace::Static(id)) =
            crate::native_closure_abi::direct_borrow_home_place(value)
        {
            let ty = static_in(self.program, id)?.ty;
            let mut result = Value::owner(Root::Static(id));
            result.homes.insert(Some(ty));
            result.types.insert(Some(ty));
            result.mark_unknown_callable(ty);
            return Ok(result);
        }
        if let Some(crate::native_closure_abi::BorrowHomePlace::Local {
            local,
            projection: crate::native_closure_abi::BorrowHomeProjection::MixedPayload(_),
        }) = crate::native_closure_abi::direct_borrow_home_place(value)
        {
            return Ok(flow.values[local.0].clone().mixed_payload());
        }
        let projected = match crate::native_closure_abi::direct_borrow_home_place(value) {
            Some(crate::native_closure_abi::BorrowHomePlace::Property { object, property }) => {
                Some((object, property))
            }
            _ => retained::stored_property(value).or_else(|| {
                let expression = crate::native_shared::Expression::from_rvalue(value)?;
                match expression.operation() {
                    crate::native_shared::Operation::Property { object, property } => {
                        Some((object, property))
                    }
                    _ => None,
                }
            }),
        };
        if let Some((object, property)) = projected {
            let mut result = Value::default();
            for owner in &flow.values[object.0].owners {
                if let Some(value) = flow.field(&owner.project(property)) {
                    result.join(&value);
                } else if property_in(self.program, property.class, property)?
                    .borrowed_source
                    .is_some()
                {
                    let mut unknown = Value::owner(Root::Unknown);
                    unknown.mark_unknown_callable(value.ty());
                    result.join(&unknown);
                } else {
                    result.owners.insert(owner.project(property));
                    if self.purpose == Purpose::Pending || flow.values[object.0].unknown_contents {
                        result.mark_unknown_callable(value.ty());
                    }
                }
            }
            if self.purpose == Purpose::Writeback {
                let definition = property_in(self.program, property.class, property)?;
                if definition.borrowed_source.is_none() {
                    result.homes = HashSet::from([Some(definition.ty)]);
                } else if result.homes.is_empty() {
                    result.homes.insert(None);
                }
                if result.types.is_empty() {
                    result.types.insert(Some(value.ty()));
                }
            }
            return Ok(result);
        }
        if let Some(expression) = crate::native_shared::Expression::from_rvalue(value) {
            return self.shared_value(context, site, flow, expression);
        }
        if let mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::NarrowedLocal { local, .. },
            ..
        }) = value
        {
            let function = function_in(self.program, self.contexts[context].function)?;
            if matches!(
                local_in(function, *local)?.ty,
                mir::Type::Mixed | mir::Type::NullableMixed
            ) {
                return Ok(flow.values[local.0].clone().mixed_payload());
            }
        }
        match value {
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(value)) => {
                self.value(context, site, flow, &mir::Rvalue::Value(value.clone()))
            }
            mir::Rvalue::NullableString(mir::NullableStringExpression::String(value)) => {
                self.value(context, site, flow, &mir::Rvalue::String(value.clone()))
            }
            mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(value)) => {
                self.value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::PayloadEnum(value.clone()),
                )
            }
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(value)) => {
                self.value(context, site, flow, &mir::Rvalue::Mixed(value.clone()))
            }
            mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Region { source, .. }) => {
                self.value(context, site, flow, source)
            }
            mir::Rvalue::ByteRegion(mir::ByteRegionExpression::AssumePresent { value, .. }) => self
                .value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::NullableByteRegion((**value).clone()),
                ),
            mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Coalesce {
                left, right, ..
            }) => {
                let left = mir::Rvalue::NullableByteRegion((**left).clone());
                let right_site = Site {
                    call: site.call + value_call_count(&left),
                    ..site
                };
                let mut result = self.value(context, site, flow, &left)?;
                result.join(&self.value(
                    context,
                    right_site,
                    flow,
                    &mir::Rvalue::ByteRegion((**right).clone()),
                )?);
                Ok(result)
            }
            mir::Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Present(value)) => {
                self.value(context, site, flow, &mir::Rvalue::ByteRegion(value.clone()))
            }
            mir::Rvalue::Class(mir::ClassExpression::InterfaceReceiver { receiver, .. })
            | mir::Rvalue::Collection(mir::CollectionExpression::InterfaceReceiver {
                receiver,
                ..
            })
            | mir::Rvalue::Value(mir::ValueExpression::Inline(
                mir::InlineValueExpression::InterfaceReceiver { receiver, .. },
            )) => Ok(flow.values[receiver.0].clone()),
            mir::Rvalue::Class(mir::ClassExpression::InterfacePayload { local, .. })
            | mir::Rvalue::Class(mir::ClassExpression::NullableLocalAssumeNonNull {
                local, ..
            })
            | mir::Rvalue::Class(mir::ClassExpression::MixedPayload { mixed: local, .. })
            | mir::Rvalue::Function(mir::FunctionExpression::MixedPayload {
                mixed: local, ..
            })
            | mir::Rvalue::Interface(mir::InterfaceExpression {
                value: mir::InterfaceValue::NarrowedLocal { local, .. },
                ..
            }) => Ok(flow.values[local.0].clone()),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                value: mir::InterfaceValue::FromClass { object, .. },
                ..
            }) => self.value(context, site, flow, &mir::Rvalue::Class((**object).clone())),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                value: mir::InterfaceValue::FromNullableClass { object, .. },
                ..
            }) => self.value(
                context,
                site,
                flow,
                &mir::Rvalue::NullableClass((**object).clone()),
            ),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                value: mir::InterfaceValue::FromValue { value, .. },
                ..
            }) => self.value(context, site, flow, value),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                value: mir::InterfaceValue::Upcast { source, .. },
                ..
            }) => self.value(
                context,
                site,
                flow,
                &mir::Rvalue::Interface((**source).clone()),
            ),
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(value)) => {
                self.value(context, site, flow, &mir::Rvalue::Class(value.clone()))
            }
            mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(
                value,
            )) => self.value(context, site, flow, &mir::Rvalue::Collection(value.clone())),
            mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                interface,
                value: mir::NullableInterfaceValue::Present(value),
            }) => self.value(
                context,
                site,
                flow,
                &mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface: *interface,
                    value: value.clone(),
                }),
            ),
            mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                value: mir::NullableInterfaceValue::Upcast { source, .. },
                ..
            }) => self.value(
                context,
                site,
                flow,
                &mir::Rvalue::NullableInterface((**source).clone()),
            ),
            mir::Rvalue::Mixed(mir::MixedExpression::BoxFunction { value, .. }) => {
                let mut result = self.value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::Function((**value).clone()),
                )?;
                result.boxed_homes =
                    HashSet::from([Some(mir::Type::Function(value.function_type()))]);
                Ok(result)
            }
            mir::Rvalue::Mixed(mir::MixedExpression::BoxClass { value, .. }) => {
                let mut result =
                    self.value(context, site, flow, &mir::Rvalue::Class(value.clone()))?;
                result.boxed_homes = HashSet::from([Some(mir::Type::Class(value.class()))]);
                Ok(result)
            }
            mir::Rvalue::Mixed(mir::MixedExpression::BoxInterface { value, .. }) => {
                let mut result = self.value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::Interface((**value).clone()),
                )?;
                result.boxed_homes = HashSet::from([Some(mir::Type::Interface(value.interface))]);
                Ok(result)
            }
            mir::Rvalue::Mixed(mir::MixedExpression::BoxPayloadEnum { value }) => {
                let mut result = self.value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::PayloadEnum((**value).clone()),
                )?;
                result.boxed_homes = HashSet::from([Some(mir::Type::PayloadEnum(value.ty()))]);
                Ok(result)
            }
            mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value)) => {
                self.value(context, site, flow, &mir::Rvalue::Function(value.clone()))
            }
            mir::Rvalue::Function(mir::FunctionExpression::AssumePresent { value, .. }) => self
                .value(
                    context,
                    site,
                    flow,
                    &mir::Rvalue::NullableFunction((**value).clone()),
                ),
            mir::Rvalue::Function(mir::FunctionExpression::Create {
                descriptor,
                captures,
                ..
            }) => {
                let own = self.contexts[context].scope;
                let home = Owner::new(Root::Environment(own, *descriptor));
                let mut values = Vec::new();
                let mut capture_site = site;
                for capture in captures {
                    values.push(match capture {
                        mir::ClosureCaptureOperand::BorrowLocal { local, .. } => {
                            flow.values[local.0].clone()
                        }
                        mir::ClosureCaptureOperand::CopyValue(value)
                        | mir::ClosureCaptureOperand::MoveValue(value) => {
                            let captured = self.value(context, capture_site, flow, value)?;
                            capture_site.call += value_call_count(value);
                            captured
                        }
                    });
                }
                let environment = self
                    .environments
                    .entry((context, *descriptor))
                    .or_insert_with(|| vec![Value::default(); values.len()]);
                let mut changed = false;
                for (old, value) in environment.iter_mut().zip(values) {
                    changed |= old.join(&value);
                }
                let closure = Closure {
                    creator: own,
                    descriptor: *descriptor,
                    home: home.clone(),
                };
                if self.purpose == Purpose::Writeback {
                    let definition = self
                        .definitions
                        .entry((context, *descriptor))
                        .or_insert_with(|| CallableDefinition {
                            closure: closure.clone(),
                            fields: HashMap::new(),
                            context: None,
                        });
                    changed |= join_fields(&mut definition.fields, &flow.fields);
                }
                if changed {
                    self.refresh_environment(context, *descriptor)?;
                }
                Ok(Value {
                    owners: HashSet::from([home.clone()]),
                    closures: HashSet::from([closure]),
                    ..Value::default()
                })
            }
            mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Call { function, args, .. })
            | mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Call {
                function, args, ..
            })
            | mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::NullableString(mir::NullableStringExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::String(mir::StringExpression::Call { function, args, .. })
            | mir::Rvalue::Value(
                mir::ValueExpression::Integer(mir::IntegerExpression::Call {
                    function, args, ..
                })
                | mir::ValueExpression::Float(mir::FloatExpression::Call { function, args, .. })
                | mir::ValueExpression::Bool(mir::BoolExpression::Call { function, args, .. })
                | mir::ValueExpression::Enum(mir::EnumExpression::Call { function, args, .. })
                | mir::ValueExpression::Inline(mir::InlineValueExpression::Call {
                    function,
                    args,
                    ..
                }),
            )
            | mir::Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::Class(mir::ClassExpression::Call { function, args, .. })
            | mir::Rvalue::Collection(mir::CollectionExpression::Call { function, args, .. })
            | mir::Rvalue::NullableClass(mir::NullableClassExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Call {
                function,
                args,
                ..
            })
            | mir::Rvalue::Mixed(mir::MixedExpression::Call { function, args, .. })
            | mir::Rvalue::Function(mir::FunctionExpression::Call { function, args, .. })
            | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Call {
                function,
                args,
                ..
            }) => self.direct_result(context, site, *function, args, flow),
            mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                value: mir::NullableInterfaceValue::Call { function, args, .. },
                ..
            }) => self.direct_result(context, site, *function, args, flow),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                value: mir::InterfaceValue::Call { function, args, .. },
                ..
            }) => self.direct_result(context, site, *function, args, flow),
            mir::Rvalue::Class(mir::ClassExpression::Coalesce { left, right, .. }) => self
                .coalesce(
                    context,
                    site,
                    flow,
                    mir::Rvalue::NullableClass((**left).clone()),
                    mir::Rvalue::Class((**right).clone()),
                ),
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Coalesce {
                left,
                right,
                ..
            }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullableClass((**left).clone()),
                mir::Rvalue::NullableClass((**right).clone()),
            ),
            mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Coalesce {
                left,
                right,
                ..
            }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullableCollection((**left).clone()),
                mir::Rvalue::NullableCollection((**right).clone()),
            ),
            mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Coalesce {
                left, right, ..
            }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullablePayloadEnum((**left).clone()),
                mir::Rvalue::PayloadEnum((**right).clone()),
            ),
            mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Coalesce {
                left,
                right,
                ..
            }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullablePayloadEnum((**left).clone()),
                mir::Rvalue::NullablePayloadEnum((**right).clone()),
            ),
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Coalesce {
                left,
                right,
                ..
            }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullableMixed((**left).clone()),
                mir::Rvalue::NullableMixed((**right).clone()),
            ),
            mir::Rvalue::String(mir::StringExpression::Coalesce { left, right }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullableString((**left).clone()),
                mir::Rvalue::String((**right).clone()),
            ),
            mir::Rvalue::NullableString(mir::NullableStringExpression::Coalesce {
                left,
                right,
            }) => self.coalesce(
                context,
                site,
                flow,
                mir::Rvalue::NullableString((**left).clone()),
                mir::Rvalue::NullableString((**right).clone()),
            ),
            mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
                place: mir::PayloadEnumPlace::CollectionIndex { collection, .. },
                ..
            })
            | mir::Rvalue::NullablePayloadEnum(
                mir::NullablePayloadEnumExpression::Use {
                    place: mir::PayloadEnumPlace::CollectionIndex { collection, .. },
                    ..
                }
                | mir::NullablePayloadEnumExpression::CollectionGet { collection, .. },
            )
            | mir::Rvalue::String(mir::StringExpression::CollectionIndex { collection, .. })
            | mir::Rvalue::NullableString(mir::NullableStringExpression::DictionaryGet {
                collection,
                ..
            }) => self.collection_value(context, flow, *collection),
            _ if value.is_null_value() => Ok(Value::default()),
            mir::Rvalue::Class(mir::ClassExpression::New {
                properties,
                args,
                span,
                ..
            }) => {
                // A call ordinal alone aliases a fresh object with its enclosing
                // aggregate, or with another constructor-free object operand.
                Ok(Value::owner(Root::Construction(
                    self.contexts[context].scope,
                    Site {
                        call: site.call
                            + properties
                                .iter()
                                .map(|property| match &property.source {
                                    mir::PropertyValueSource::Expression(value) => {
                                        value_call_count(value)
                                    }
                                    _ => 0,
                                })
                                .sum::<usize>()
                            + args.iter().map(value_call_count).sum::<usize>(),
                        ..site
                    },
                    *span,
                )))
            }
            mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Construct { .. }) => Ok(
                Value::owner(Root::Temporary(self.contexts[context].scope, site)),
            ),
            _ if value.ty().has_move_ownership() => Ok(Value::owner(Root::Temporary(
                self.contexts[context].scope,
                site,
            ))),
            _ if byte_regions::kind(value.ty()).is_some() => Ok(Value::owner(Root::Unknown)),
            _ => Ok(Value::default()),
        }
    }

    fn coalesce(
        &mut self,
        context: usize,
        site: Site,
        flow: &Flow,
        left: mir::Rvalue,
        right: mir::Rvalue,
    ) -> Result<Value, BackendError> {
        let right_site = Site {
            call: site.call + value_call_count(&left),
            ..site
        };
        let mut value = self.value(context, site, flow, &left)?;
        value.types.remove(&None);
        value.join(&self.value(context, right_site, flow, &right)?);
        Ok(value)
    }

    fn collection_value(
        &self,
        context: usize,
        flow: &Flow,
        local: mir::LocalId,
    ) -> Result<Value, BackendError> {
        self.collection_projection(context, flow, local, Projection::Element(None))
    }

    fn collection_projection(
        &self,
        context: usize,
        flow: &Flow,
        local: mir::LocalId,
        projection: Projection,
    ) -> Result<Value, BackendError> {
        let function = function_in(self.program, self.contexts[context].function)?;
        let ty = local_in(function, local)?.ty;
        let collection = match ty {
            mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
                collection
            }
            _ => return Err(malformed_mir("indexed storage has no collection type")),
        };
        let definition = collection_in(self.program, collection)?;
        let stored = if projection == Projection::Key {
            definition
                .key
                .ok_or_else(|| malformed_mir("collection has no key storage"))?
        } else {
            definition.value
        };
        let mut value = Value::default();
        for owner in &flow.values[local.0].owners {
            let selected = owner.projected(projection.clone());
            if let Some(stored) = flow.field(&selected) {
                value.join(&stored);
            } else if flow.values[local.0].unknown_contents {
                value.mark_unknown_callable(stored);
            }
            value.owners.insert(selected);
        }
        value.homes = HashSet::from([Some(stored)]);
        if value.types.is_empty() {
            value.types.insert(Some(stored));
        }
        Ok(value)
    }

    fn shared_value(
        &mut self,
        context: usize,
        site: Site,
        flow: &Flow,
        expression: crate::native_shared::Expression<'_>,
    ) -> Result<Value, BackendError> {
        use crate::native_shared::Operation as O;
        match expression.operation() {
            O::Local { local, .. } => Ok(flow.values[local.0].clone()),
            O::Call { function, args, .. } => {
                self.direct_result(context, site, function, args, flow)
            }
            O::Present(value) => self.shared_value(context, site, flow, value),
            O::Coalesce { left, right, .. } => {
                let mut accesses = ClassLocalAccesses::default();
                collect_shared_handle_local_accesses(left, &mut accesses);
                let right_site = Site {
                    call: site.call + accesses.direct_calls,
                    ..site
                };
                let mut value = self.shared_value(context, site, flow, left)?;
                value.types.remove(&None);
                value.join(&self.shared_value(context, right_site, flow, right)?);
                Ok(value)
            }
            O::Index { collection, .. } | O::Get { collection, .. } => {
                self.collection_value(context, flow, collection)
            }
            // New control blocks and access registrations have their own slot.
            O::New { .. } | O::Runtime { .. } => Ok(Value {
                homes: HashSet::from([Some(expression.ty())]),
                types: HashSet::from([Some(expression.ty())]),
                ..Value::owner(Root::Temporary(self.contexts[context].scope, site))
            }),
            O::Null => Ok(Value {
                types: HashSet::from([None]),
                ..Value::default()
            }),
            O::Property { .. } => {
                unreachable!("shared properties handled before operation projection")
            }
        }
    }
}
