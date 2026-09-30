<script lang="ts">
  import BrowserTabPanel from '../../src/lib/components/BrowserTabPanel.svelte';

  let tab = $state({ id: 'one', url: 'https://one.example/start', title: 'One', unloaded: false });
  let nativeUrl = 'https://one.example/start';
  let submitted = $state('');
  setInterval(() => { tab = { ...tab, url: nativeUrl }; }, 40);

  function setNativeUrl(url: string) { nativeUrl = url; }
  (window as Window & { setNativeUrlForTest: (url: string) => void }).setNativeUrlForTest = setNativeUrl;
  function switchTab() {
    nativeUrl = 'https://two.example/current';
    tab = { id: 'two', url: nativeUrl, title: 'Two', unloaded: false };
  }
</script>

<button onclick={() => setNativeUrl('https://one.example/polled')}>Poll native URL</button>
<button onclick={switchTab}>Switch browser tab</button>
<output aria-label="Submitted address">{submitted}</output>
<BrowserTabPanel {tab} onNavigate={value => { submitted = value; nativeUrl = value; tab = { ...tab, url: value }; }} onLayout={() => {}} />
