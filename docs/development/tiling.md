# Tiling state and resource ownership

For a visual introduction with interactive ownership, layout, command-routing,
and request-lifetime examples, read the [Tiling architecture chapter](../html/tiling.html).
The HTML chapter opens directly from disk and works offline.

`Workspace` owns the layout, tile entries, and shared item lists. Its storage is
private to `tiles::workspace` and its implementation modules. Renderers, input
handlers, and other application code use the immutable `layout()`, `tiles()`, and
`item_lists()` accessors. They cannot independently change layout membership or
remove a tile's resources.

## Mutation boundary

The application dispatcher records semantic history around workspace commands.
Layout adapters submit revision-checked proposals through `apply_layout_edit`.
Workspace commands handle creation, splitting, moving, closing, and renaming.
The egui adapter keeps tab bars around panes but prunes obsolete tab wrappers
around splits. Drops that would nest a split inside tabs are translated into
semantic moves: a pane-edge drop splits the destination tab group, and a tab
drop onto a split joins its nearest visible pane. The persistent layout still
allows only tile leaves inside tabs. Invalid runtime trees are discarded and
rebuilt from the validated workspace, and submitted proposals are reconciled
on the next frame even if the dispatcher ignored or rejected them.
Tile messages and dedicated document/list operations handle settings, navigation,
source attachment, marker edits, and restoration of list content. List edits
repair the references and caches of surviving views sharing that content.

Creation and splitting stage an entry and all new resources before installing
them. The prepared insertion operation allocates a fresh tile identity and
checks placement, singleton constraints, resource identities, dependencies, and
view references before committing storage. Errors leave the live workspace and
layout revision unchanged. Identity counters may advance during failed staging;
allocated identities are never recycled.

History and legacy migration are workspace implementation modules. History owns
resources absent from the live workspace, validates restored membership and
resource dependencies before swapping them back, and resets restored runtime
caches. Surviving linked views retain their resource identity and navigation.
The runtime allocator and request epochs are never restored by undo.

Workspace replacement validates a complete candidate before installing its IDs,
advancing the workspace epoch, and replacing the live aggregate. Document
replacement advances the document generation. Async consumers must check the
current owner, input key, and request token; neither undo nor copying restores
pending runtime work.

## Dependencies

Each tile kind declares `Dependencies`: a set of typed `ResourceId` references
and whether additional references are opaque. The current resource type is
`ResourceId::ItemList(ItemListId)`. There is no assumption that a tile owns at
most one resource. Repeated references resolve to the same resource.

The dependency model is used for:

- Load and insertion validation, including all known references even when an
  unknown tile is present.
- Resource collection after closing tiles or resetting the workspace.
- Determining which resources a history record must retain and validating
  restoration.
- Independent copying and complete reference remapping.

Linked copies keep the original references. Independent copies copy each
distinct dependency once, allocate new resource identities, and remap the
copied owner's references. The copy algorithm verifies that the remapped
owner declares exactly the copied dependencies. It rejects missing resources,
incomplete remapping, and opaque dependencies before installing anything.
Copied item lists contain content, not layout caches.

`waveform_list()` is a waveform-specific accessor for editing its primary list.
It must not be used to infer ownership or decide resource lifetime. A new tile
with several lists declares every list in `dependencies()` and remaps every
reference in `ResourceOwner::remap_resources`. Kind-specific validation checks
any references into the resource contents. Add a new `ResourceId` variant and
its concrete storage/copy operations only when another resource type exists.

## Saved state and unknown kinds

This cleanup does not change `WorkspaceFile` version 1, tile payload versions,
or the version-zero migration. Resource references remain in the existing
kind payloads, and item lists remain in their existing saved collection.

Unknown kinds and unsupported payload versions retain their raw RON payload.
Their dependencies are opaque, so collection conservatively keeps all resource
storage while any such tile survives. Closing the last opaque owner allows
collection to resume; undo retains and restores the resources removed by that
operation. Opaque retention does not excuse a known tile's missing dependency.
Unknown tiles cannot be independently copied because their references cannot
be safely rewritten.

## Single-tile presentation

By default, a workspace with one tile hides its tab bar and focus outline.
The default waveform uses 100-pixel name and value columns, matching the
pre-tiling layout; an empty waveform shows the welcome screen. The toolbar
keeps the compact add/remove controls until a second tile is opened. Multiple
tiles show tab bars, focus outlines, and directional split controls. Set
`layout.hide_single_tab_bar = false` to show tile controls even for one tile.
