<script>
  import CommandPalette from '../../src/lib/components/CommandPalette.svelte';

  let jev = $state({});
  const qa = window.__PALETTE_JEV_QA__ = {
    interpreted: null,
    selected: null,
    executions: 0,
  };

  function interpret(query) {
    qa.interpreted = query;
    jev = { loading: true };
    setTimeout(() => {
      jev = {
        suggestion: {
          candidateId: 'scale-down',
          label: 'Decrease interface scale',
          detail: 'Jev · 91% · Matches the request to make text smaller',
        },
      };
    }, 20);
  }

  function select(id) {
    qa.selected = id;
    if (id.startsWith('__jev-suggestion:')) qa.executions += 1;
  }
</script>

<CommandPalette
  open={true}
  title="Controls"
  placeholder="Find a control or setting…"
  items={[
    { id: 'scale-down', label: 'Decrease interface scale', group: 'Appearance', keywords: 'zoom smaller' },
    { id: 'theme:dark', label: 'Dark theme', group: 'Appearance' },
  ]}
  {jev}
  oninterpret={interpret}
  onselect={select}
  onclose={() => {}}
/>
