# Doria application package design

**Role:** Supporting design exploration. The [end-to-end plan](doria-end-to-end-plan.md#21-application-package-checkpoints) owns sequencing, prerequisites, and acceptance gates. This document proposes package architecture; exact APIs, source forms, renderer choices, and host ABIs are not frozen or implemented.

## Product boundaries

The target is pleasant application authoring over a native Doria core. Components, stylesheets, evidence models, and language adapters belong in packages. Ownership, checked errors, async, and FFI remain general language mechanisms. The terminal and graphical frameworks share component/state/event principles where useful but keep separate cell-grid and graphical renderers.

Package namespaces are `Doria\Tui` and `Doria\Osint`. `Doria\Ui` is a provisional namespace for the graphical framework, pending product naming. All package implementations live in separate repositories. No new component grammar, stylesheet grammar, or OSINT keywords are proposed for the compiler.

## `Doria\Tui`

### First application

A searchable list with a detail panel, input field, status bar, scrolling, and keyboard navigation. It is useful as an Ichiloto editor panel or an investigation triage interface. Use one source on Linux, macOS, and Windows, with PTY/ConPTY acceptance evidence.

### Proposed runtime model

- The application owns its model and root component. An update operation consumes a typed event and changes state explicitly; view construction derives component output from that state.
- Components have stable keys, typed props, explicit event subscriptions, and scoped state. Removed components cancel their tasks and subscriptions and release owned resources. Use weak references for back-links; do not create strong ownership cycles.
- A terminal event loop consumes Stage 36a readiness and Stage 46 key/resize events. A synchronous application uses that substrate without an async scheduler. Async applications reuse Stage 38–39 structured tasks and cancellation. There is one readiness model and bounded queues.
- Layout resolves constraints into terminal cells. The first layout surface supports rows, columns, padding, alignment, clipping, and scrolling. The initial widgets are text, container, list, input field, and status bar.
- A package-owned cell surface computes changed runs and sends them through `Console`. The standard library remains stateless; frame buffers and diffing are package concerns. Account for graphemes, combining marks, wide glyphs, tabs, and terminal-dependent width. A public width policy and fallback behavior need a decision before release.
- Focus and shortcut routing are deterministic. Input editing works on graphemes rather than arbitrary UTF-8 byte offsets. Render only after state, terminal size, or animation deadlines change; ordinary idle work does not busy-poll.
- Raw mode and its restoring guard remain owned by `Doria\Std\Term`. The package releases sessions on structured exits and checked errors. Abort-only panics retain the existing runtime policy; panic restoration requires its own runtime decision.
- Themes expose semantic colors and text attributes. User components never carry ANSI escapes. Plain-text output and redirected-stream behavior are explicit alternatives for automation and accessible consumption.

### Verification and design rulings

Headless layout and cell snapshots cover Unicode, clipping, focus, scrolling, and resize. Platform harnesses check input decoding, redirected output, session restoration, and checked-error exits. Performance cases assert bounded writes, reuse of readiness storage, and no mandatory allocation for every unchanged frame; record timing only on eligible runners.

Before a public API, settle component identity, state/callback ownership, Unicode width, event priority and backpressure, text-editing behavior, accessibility/fallback behavior, and terminal-session exclusivity. This is package design work against the accepted stream foundation; executable portability depends on Stage 46. API illustrations must be marked as proposals until their package decision is accepted.

## `Doria\Osint`

### First investigation

Collect one public JSON API and one RSS/Atom feed. Capture the response bytes and collection metadata, extract observations, propose a few entity matches, explicitly review a finding, then export a cited report. Disconnect the network and replay the same investigation from the captured sources.

The first store is a local snapshot directory and append-only manifest. A database, OpenSearch, graph engine, broad connector catalogue, OCR, and browser automation can be adapters later; they do not gate the first prototype.

### Evidence workflow

- **Capture:** preserve exact raw response bytes, content hash, source URL, collection timestamp, response status, relevant headers, collector version, and request configuration. Keep credentials out of exported metadata. Distinguish source-provided time from collection time.
- **Observe:** store the extraction version and a source selector or byte/text location that links an observation to its snapshot. Normalization retains the original value and the transformation history.
- **Propose:** represent entity matches as candidates with supporting and contradicting observations, reasoning, and uncertainty. Confidence is an analyst/model assessment, not a truth guarantee.
- **Review:** publication accepts an explicit reviewed finding rather than an arbitrary candidate. Package boundaries control ordinary construction and transitions; negative examples verify that the safe public API cannot publish a candidate directly. Review records actor, rationale, time, and evidence revision. Unsafe code, foreign hosts, and external edits remain trust boundaries.
- **Replay:** version the input snapshots, transform code, configuration, and toolchain identity. Replay performs no acquisition; network attempts fail explicitly. Hashes detect byte modification, but neither hashes nor types certify that a source was truthful. Record nondeterministic dependencies instead of claiming reproducibility for them.
- **Export:** each report claim links to its observation and captured source. Provide a machine-readable manifest plus a human report; preserve candidate/review status. Tamper-evident hashes are distinct from legally established chain of custody.

General HTTP, TLS, JSON, time, hashes, and I/O remain standard-library or reusable package capabilities. The OSINT package owns acquisition policies, source/evidence relationships, review transitions, replay manifests, and report generation. Raw archives survive a parser failure so an engineer can correct the transform without reacquiring a changed source.

### The adoption test

Implement the same investigation in Doria, Python, Rust, and Go with the same input fixtures, policy, output schema, and failure cases. Give agents equivalent tasks and resources, record the assistance they used, and have reviewers assess anonymized outputs where practical. Mature provenance libraries are valid competitor choices; do not compare Doria with deliberately incomplete scripts.

Measure human time to a verified report, broken-source recovery, adding a source, tracing a claim, offline replay, deployment, and maintenance. Record correctness, evidence gaps, bounded memory, and failure recovery separately from speed. Include timeout, rate limit, pagination, truncated response, invalid encoding, schema drift, duplicate entity, changed snapshot, and cancellation cases. There is no preselected winner: a useful result may identify a missing Doria foundation or show that existing tools remain preferable.

## Native component UI framework

### Target authoring experience

A component combines a view, a stylesheet file, and business logic in the selected language. Typed props and events connect components. The initial Doria application has a search box, virtualized results list, details panel, and cancellable loading. A PHP component implements the same business behavior after its host boundary is proven.

[GPUI](https://gpui.rs/) is a reference for native rendering, composable elements, and CSS-inspired layout. It describes a hybrid immediate/retained model and GPU acceleration. These are reference inputs, not a requirement to duplicate its Rust API, adopt its backend, or claim its performance. The renderer decision must compare viable native window/graphics/text stacks before selection.

### Proposed framework layers

- **Component runtime:** stable identity, props, owned state, lifecycle, explicit events, subscriptions, scheduling, and incremental reconciliation. State changes invalidate affected layout/paint work. A component removal cancels its outstanding effects.
- **Style and layout:** component-scoped selectors, tokens/custom properties, a documented cascade, sizing, spacing, typography, colors, focus/disabled states, and one well-defined initial layout model. Prefer a bounded CSS subset with source-mapped errors. Browser DOM, browser CSS completeness, and JavaScript execution are separate product choices.
- **Scene and rendering:** a native-owned graphical tree, clipping, paint invalidation, images, text shaping, and reusable graphics resources. A deterministic headless backend supports layout and event tests. GPU/backend selection is an open decision, not a compiler feature.
- **Platform services:** windows, pointer/keyboard input, IME, clipboard, dialogs, fonts, DPI scaling, and accessibility. Cross-platform semantics must be reviewed and exercised on Linux, macOS, and Windows.
- **Host adapters:** Doria first, PHP as a deliberate supported path, and additional languages through a versioned host-neutral component/event boundary when a real consumer establishes the requirements. Do not export Doria object layout, runtime pointers, or language-specific ownership wrappers as a public ABI.
- **Build tooling:** package tooling discovers component/style/assets and invokes reusable parsers and `doriac`; it does not add UI parsing to the compiler. Hot reload is a later package capability after ownership/lifecycle rules are proven.

### PHP and other languages

Keep the hosting direction explicit. Stage 41 enables PHP calling generated Doria exports. A PHP-hosted UI can explore that path, but event callbacks, UI thread ownership, reentrancy, and lifetime rules still require review.

A standalone native Doria application running PHP business logic requires the separately deferred embedded-PHP host. Its contract must cover thread-affine PHP values, callback dispatch, checked error mapping, native handle lifetimes, shutdown, and worker isolation. Native async workers exchange validated data and return to the PHP-owning thread for PHP execution. Stage 41 alone is not completion of this host.

The first additional-language adapter should prove one component, events, async results, errors, and teardown through the same transport-neutral protocol. A promise that arbitrary languages can directly execute callbacks is outside the first framework slice.

### Acceptance and unresolved choices

The Doria vertical slice must demonstrate component composition, styles, keyboard/pointer navigation, IME, resize/DPI, accessibility inspection, async cancellation, and deterministic teardown on all tier-1 hosts. Test malicious or malformed text/style input and large lists. PHP authoring has a separate demonstrator gate for its actual hosting direction.

Before implementation, choose the rendering/window stack, stylesheet subset and cascade, component/reactivity model, accessibility provider, host protocol, distribution format, and text/IME policy. Compare the same application with an established framework for authoring/review effort, startup, memory, and update latency. The compiler's bounded UI demo remains the existing 1.0 requirement; shipping the full framework is a separate product milestone.

## Invalidated elsewhere

- Initial network/HTTP/TLS acquisition now belongs to Stage 40a; Stage 44 matures and hardens it. Decision 0110, the stdlib inventory, and the native Baton dependency description must use that split.
- The old UI product description limited the framework to integration with PHP web backends. The target now includes native components, stylesheets, Doria logic, and a reviewed PHP host path.
- The TUI widget layer remains userland, now with `Doria\Tui` as a named consumer; `Doria\Std\Term` stays primitive and Stage 46 remains its owner.
- No compiler syntax, implemented API, backend behavior, IDE fact, installed binary, or website example changes in this design beat. Future accepted APIs will require the ordinary compiler/tooling/reference/website synchronization.
