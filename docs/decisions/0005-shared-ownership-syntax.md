# 0005 Shared ownership syntax

Status: Accepted (amended — see "Stage 25a reconciliation")

## Decision

The surface syntax for shared ownership is:

```doria
shared new AppConfig(...)
```

The likely explicit type form is:

```doria
shared AppConfig $config = shared new AppConfig(...);
```

## Notes

`shared` is a Doria ownership modifier, not a Rust-style wrapper exposed as primary syntax. Shared ownership should not automatically imply shared mutation. Weak ownership remains an open question.

## Stage 25a reconciliation

The original decision above is preserved as authored. This section, added when the
Stage 25a shared-ownership types are authored, fixes how `shared new` relates to
the static type system — which the original wording left implicit — without
rewriting the historical record. `SharedReference<T>` did not exist when this
decision was written; the reconciliation is forward-looking.

- **`shared new T(...)` is the construction spelling for the Stage 25a
  `SharedReference<T>` type.** The expression creates the object directly under
  shared ownership and has static type `SharedReference<T>`. It is *not* specified
  as "create an owned `T`, then implicitly wrap it," and there is no implicit
  conversion from an owned `T` into `SharedReference<T>`.

  ```doria
  let $node = shared new Node();                     // $node : SharedReference<Node>
  SharedReference<Node> $node = shared new Node();    // explicit form
  ```

- **`shared` is a construction modifier only.** It is not itself a type, and it is
  not a general binding, property, parameter, method, or class modifier. The
  original "likely explicit type form" `shared AppConfig $config = ...` is
  **superseded**: shared ownership in signatures and stored declarations is written
  through the generic type `SharedReference<T>`, never a `shared`-prefixed
  declaration.

  ```doria
  SharedReference<AppConfig> $config = shared new AppConfig(...);
  ```

- The corresponding non-owning and runtime-checked-writable forms are the generic
  types `WeakReference<T>` and `WritableSharedReference<T>`. Their construction and
  access surface is settled by the Stage 25a shared-ownership decision, not here.
  `shared new` always produces `SharedReference<T>`; it never contextually produces
  `WritableSharedReference<T>` or another ownership type, and this reconciliation
  introduces no `weak new` / `writable shared new` modifier chain.
