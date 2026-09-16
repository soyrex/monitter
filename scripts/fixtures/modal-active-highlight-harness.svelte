<script>
  import '../../src/app.css';
  import PaneGrid from '../../src/lib/components/PaneGrid.svelte';
  import Modal from '../../src/lib/components/Modal.svelte';
  import CommandPalette from '../../src/lib/components/CommandPalette.svelte';
  import ImageLightbox from '../../src/lib/components/ImageLightbox.svelte';
  import ShareControl from '../../src/lib/components/ShareControl.svelte';

  const layout = { id: 'primary-pane' };
  let showActivePaneBorder = $state(true);
  let modalOpen = $state(false);
  let paletteOpen = $state(false);
  let lightboxImage = $state(null);
  let sharingOpen = $state(false);
  const fixtureImage = {
    src: 'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" width="640" height="360"%3E%3Crect width="640" height="360" fill="%2300a8f0"/%3E%3C/svg%3E',
    alt: 'Fixture blue image',
    title: 'Fixture image preview',
  };
</script>

<main class="fixture-shell">
  <div class="fixture-controls">
    <label><input type="checkbox" bind:checked={showActivePaneBorder} /> Show active pane border</label>
    <button type="button" onclick={() => (modalOpen = true)}>Open fixture modal</button>
    <button type="button" onclick={() => (sharingOpen = true)}>Open sharing</button>
  </div>

  <PaneGrid
    {layout}
    activePaneId="primary-pane"
    {showActivePaneBorder}
    dimInactivePanes={false}
    onactivate={() => {}}
    onresize={() => {}}
    ondropTab={() => {}}
  >
    {#snippet children()}
      <div class="fixture-pane-content">A real PaneGrid leaf</div>
    {/snippet}
  </PaneGrid>
</main>

<Modal title="Fixture modal" open={modalOpen} onclose={() => (modalOpen = false)}>
  <p>Modal content used only by the browser test.</p>
  <button type="button" onclick={() => (paletteOpen = true)}>Open command palette</button>
</Modal>

<CommandPalette
  open={paletteOpen}
  title="Fixture commands"
  placeholder="Find a fixture command"
  items={[{ id: 'image:fixture', label: 'Preview fixture image', group: 'Fixture' }]}
  onselect={(id) => { if (id === 'image:fixture') lightboxImage = fixtureImage; }}
  onclose={() => (paletteOpen = false)}
/>

<ImageLightbox bind:image={lightboxImage} />
<ShareControl bind:open={sharingOpen} />


<style>
  :global(:root) {
    --accent: #00a8f0;
    --accent-ink: #0089c8;
    --paper: #f4f7fa;
    --panel: #ffffff;
    --soft: #edf2f6;
    --line: #b8c4cf;
    --ink: #17212b;
    --muted: #536270;
    --mono: ui-monospace, SFMono-Regular, Menlo, monospace;
    --interface-font-ratio: 1;
  }
  :global(body) { margin: 0; min-height: 100vh; }
  .fixture-shell { display: grid; grid-template-rows: auto minmax(0, 1fr); width: 100vw; height: 100vh; }
  .fixture-controls { display: flex; gap: 12px; align-items: center; padding: 12px; border-bottom: 1px solid var(--line); background: var(--panel); }
  .fixture-controls button { padding: 7px 10px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink); background: var(--soft); }
  .fixture-pane-content { padding: 24px; color: var(--ink); }
</style>
