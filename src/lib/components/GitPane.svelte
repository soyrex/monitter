<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { RefreshCw, FileText } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import type { GitDiffScope, TaskGitDiff, TaskGitStatus } from '$lib/types';

  type GitPaneStatus = { repository: boolean | null; error: string; loading: boolean; status: TaskGitStatus | null };
  let { taskId, active = false, onRepository, onStatus }: { taskId: string; active?: boolean; onRepository?: (repository: boolean) => void; onStatus?: (status: GitPaneStatus) => void } = $props();
  const bridge = getBridge();
  let status = $state<TaskGitStatus | null>(null), loading = $state(false), error = $state('');
  let selected = $state<{ path: string; scope: GitDiffScope } | null>(null), diff = $state<TaskGitDiff | null>(null), diffLoading = $state(false), diffError = $state('');
  let generation = 0, statusRevision = 0, diffRevision = 0, timer: ReturnType<typeof setInterval> | undefined;
  function publish() { onStatus?.({ repository: status?.repository ?? null, error, loading, status }); }
  export function refreshStatus() { return refresh(); }
  async function loadDiff(selection = selected, expectedGeneration = generation) {
    if (!selection) return; const current = ++diffRevision; diffLoading = true; diffError = '';
    try { const next = await bridge.getTaskGitDiff(taskId, selection.path, selection.scope); if (expectedGeneration === generation && current === diffRevision) diff = next.repository ? next : null; }
    catch (reason) { if (expectedGeneration === generation && current === diffRevision) diffError = reason instanceof Error ? reason.message : String(reason); }
    finally { if (current === diffRevision) diffLoading = false; }
  }
  async function refresh() {
    const expectedGeneration = generation, current = ++statusRevision; loading = true; error = ''; publish();
    try {
      const next = await bridge.getTaskGitStatus(taskId);
      if (expectedGeneration !== generation || current !== statusRevision) return;
      status = next; onRepository?.(next.repository); publish();
      if (!next.repository) { selected = null; diff = null; }
      else if (selected) { const file = next.files.find(file => file.path === selected?.path); const valid = file && (selected.scope === 'staged' ? !!file.indexStatus.trim() && !file.untracked : selected.scope === 'untracked' ? file.untracked : !!file.worktreeStatus.trim() && !file.untracked); if (valid) void loadDiff(selected, expectedGeneration); else { selected = null; diff = null; } }
    } catch (reason) { if (expectedGeneration === generation && current === statusRevision) { error = reason instanceof Error ? reason.message : String(reason); publish(); } }
    finally { if (current === statusRevision) { loading = false; publish(); } }
  }
  async function choose(path: string, scope: GitDiffScope) { selected = { path, scope }; diff = null; await loadDiff(selected); }
  $effect(() => { const id = taskId; void id; generation++; statusRevision++; diffRevision++; status = null; error = ''; loading = false; diffLoading = false; selected = null; diff = null; diffError = ''; untrack(() => { void refresh(); }); });
  $effect(() => { if (timer) clearInterval(timer); timer = active ? setInterval(() => { if (!loading && !diffLoading) void refresh(); }, 10_000) : undefined; return () => { if (timer) clearInterval(timer); }; });
  onDestroy(() => { generation++; statusRevision++; diffRevision++; if (timer) clearInterval(timer); });
  const files = $derived(status?.repository ? status.files ?? [] : []);
  const changed = (value: string) => value.trim().length > 0;
  const group = (scope: GitDiffScope) => files.filter(file => scope === 'staged' ? changed(file.indexStatus) && !file.untracked : scope === 'unstaged' ? changed(file.worktreeStatus) && !file.untracked : file.untracked);
</script>

<section class="git-pane" aria-label="Git changes">
  <header><span>{status?.repository && status.branch ? status.branch : 'Git'}</span><button class="icon" aria-label="Refresh Git status" title="Refresh" onclick={refresh} disabled={loading}><RefreshCw class={loading ? 'spin' : ''} size={14}/></button></header>
  {#if error}<p class="git-note error">{error} <button onclick={refresh}>Retry</button></p>
  {:else if !status || loading && !status}<p class="git-note">Checking repository…</p>
  {:else if !status.repository}<p class="git-note">No Git repository for this task.</p>
  {:else}<div class="git-body"><aside class="git-files">
    <small>{status.root}</small>
    {#each [['staged','Staged'],['unstaged','Changes'],['untracked','Untracked']] as [scope,label]}
      {@const entries = group(scope as GitDiffScope)}
      {#if entries.length}<h4>{label} <span>{entries.length}</span></h4>{#each entries as file}<button class:chosen={selected?.path===file.path && selected?.scope===scope} onclick={()=>choose(file.path, scope as GitDiffScope)}><FileText size={12}/><code>{file.path}</code></button>{/each}{/if}
    {/each}
    {#if !files.length}<p class="git-note">Working tree is clean.</p>{/if}
    {#if status.truncated}<p class="git-note">File list truncated.</p>{/if}
  </aside><article class="git-diff" aria-live="polite">
    {#if !selected}<p class="git-note">Select a changed file to inspect its diff.</p>
    {:else if diffLoading}<p class="git-note">Loading diff…</p>
    {:else if diffError}<p class="git-note error">{diffError}</p>
    {:else if diff && diff.repository && diff.binary}<p class="git-note">This file is binary and cannot be shown as a text diff.</p>
    {:else if diff && diff.repository}<pre>{#each diff.text.split('\n') as line}<span class:plus={line.startsWith('+') && !line.startsWith('+++')} class:minus={line.startsWith('-') && !line.startsWith('---')}>{line || ' '}</span>{/each}</pre>{#if diff.truncated}<p class="git-note">Diff truncated.</p>{/if}{:else}<p class="git-note">The repository or change is no longer available. Refresh to check again.</p>{/if}
  </article></div>{/if}
</section>

<style>
.git-pane{display:grid;grid-template-rows:auto minmax(0,1fr);min-height:0;height:100%;font-size:11px}.git-pane header{display:flex;justify-content:space-between;align-items:center;padding:8px;border-bottom:1px solid var(--line)}.git-body{display:grid;grid-template-rows:minmax(90px,40%) minmax(0,1fr);min-height:0;overflow:hidden}.git-files{overflow:auto;border-bottom:1px solid var(--line);padding:6px}.git-files h4{margin:10px 4px 3px;font-size:10px;color:var(--muted)}.git-files h4 span{float:right}.git-files button{display:flex;width:100%;gap:5px;padding:4px;text-align:left}.git-files button.chosen{background:var(--soft)}.git-files code{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.git-diff{overflow:auto;min-width:0}.git-diff pre{margin:0;padding:8px;user-select:text;white-space:pre;tab-size:2}.git-diff pre span{display:block}.plus{color:#237b4b;background:#237b4b14}.minus{color:#b54a55;background:#b54a5514}.git-note{padding:10px;color:var(--muted)}.git-note.error{color:#b54a55}.git-note button{text-decoration:underline}:global(.spin){animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@media(prefers-reduced-motion:reduce){:global(.spin){animation:none}}
</style>
