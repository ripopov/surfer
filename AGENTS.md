# Working on Surfer

## Repository goal

Build Surfer in Rust and document every feature in a modern, interactive HTML
book. Each feature gets one page in `docs/html/`, named for that feature (for
example, `docs/html/tiling.html`). Documentation is part of the implementation:
add or update the corresponding chapter when adding or changing a feature.

## Standard for feature chapters

- Write exceptional explanations: begin with the user's problem, explain the
  architecture and state ownership, then show behavior, tradeoffs, limitations,
  and extension points. Give readers a coherent story, not an API inventory.
- Read the Rust implementation and relevant tests before writing. Link to the
  source files supporting the explanation. Clearly distinguish current behavior
  from proposed designs and future work; never present a mockup as a shipped UI.
- Visualize data structures, identities, references, ownership, and lifetimes.
  Use concrete examples that keep the same identities across diagrams.
- Explain algorithms in plain English and visually, with step-through sequences
  or interactive demonstrations. Do not paste implementation code or pseudocode.
  Data structure declarations are the only code snippets allowed in the book.
- Make interaction teach something: let readers change inputs and see the
  consequences. Provide clear initial states, labels, feedback, and reset controls.
  Use animation where it clarifies transitions; provide pause/manual controls and
  respect `prefers-reduced-motion`. Essential explanations must remain available
  without animation or JavaScript.
- Treat pages as a consistent book: chapter identity, section navigation, readable
  typography, a deliberate visual hierarchy, and links from existing documentation.
  Use `tiling.html` as the initial editorial and visual reference.
- Keep the book portable. Prefer semantic HTML, CSS, and small vanilla JavaScript
  demonstrations that open directly from disk and work offline. Avoid mandatory
  build tools, remote fonts, CDNs, analytics, or network dependencies.
- Support narrow screens, keyboard navigation, visible focus, sufficient contrast,
  meaningful accessible names, and useful print output. Never rely on color alone.
- Validate links and exercise each interaction, including reset and boundary
  conditions. Inspect desktop and narrow layouts when browser tools are available;
  report verification limits honestly. Documentation-only changes do not need a
  Rust rebuild unless they affect Rust or its build configuration.

## Architecture documentation

Preserve the distinction between shared document data, workspace-owned resources,
tile-local state, and disposable runtime state. Explain how that separation can
support Surfer's evolution into a Verdi-like integrated debug platform, while
identifying the additional design data, cross-view contracts, and capabilities
that still need to be built. Keep trace data and design/presentation semantics
separate when discussing VTR and VDB.

## Scope and maintenance

Follow applicable parent repository instructions. Preserve unrelated work and
existing documentation. Keep changes focused, verify factual claims against this
checkout, and update source references and interactive models when Rust behavior
changes.
