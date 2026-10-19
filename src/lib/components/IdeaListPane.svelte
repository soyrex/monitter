<script module lang="ts">
  // Local drafts survive pane unmounts without entering shared backend state.
  const retainedDrafts = $state<Record<string, import('../project-ideas').IdeaDraft | undefined>>({});
  const retainedTags = $state<Record<string, string>>({});
  const retainedSaving = $state<Record<string, boolean>>({});
</script>
<script lang="ts">
  import { untrack } from 'svelte';
  import { getBridge } from '../bridge';
  import type { Project, Snapshot } from '../types';
  import { editIdeaDraft, ideaSaveList, ideaStatuses, newIdeaDraft, parseIdeaTags } from '../project-ideas';
  import Markdown from './Markdown.svelte';
  let { project, onUpdate, draftKey = 'default', onsaving }: { project: Project; onUpdate: (snapshot: Snapshot) => void; draftKey?: string; onsaving?: (saving: boolean) => void } = $props();
  // AppSurface keys this component by pane draft identity. Read its initial key once.
  const cacheKey = untrack(() => `${draftKey}:${project.id}`);
  // Keep each project's unsaved document while navigating between panes/projects.
  const drafts = retainedDrafts;
  const tags = retainedTags;
  let preview = $state(false);
  let saving = $derived(retainedSaving[cacheKey] ?? false);
  // A remounted pane observes the original mutation until it settles.
  $effect(() => { onsaving?.(saving); });
  let error = $state('');
  let draft = $derived(drafts[cacheKey]);
  let changedElsewhere = $derived(draft && draft.revision !== (project.ideasRevision ?? 0));
  function hasEdits() {
    const original = project.ideas?.find(idea => idea.id === draft?.idea.id);
    return !!draft && (!original || JSON.stringify(original) !== JSON.stringify(draft.idea) || (tags[cacheKey] ?? '') !== (original.tags ?? []).join(', '));
  }
  function capture() { drafts[cacheKey] = newIdeaDraft(project); tags[cacheKey] = ''; error = ''; preview = false; }
  function select(id: string) {
    if (saving) return;
    if (hasEdits()) { error = 'Save or discard the current draft before opening another idea.'; return; }
    const idea = project.ideas?.find(item => item.id === id);
    if (idea) { drafts[cacheKey] = editIdeaDraft(project, idea); tags[cacheKey] = (idea.tags ?? []).join(', '); error = ''; preview = true; }
  }
  async function save() {
    if (!draft || saving) return;
    const projectId = project.id;
    const savingKey = cacheKey;
    try {
      const bridge = getBridge();
      if (!bridge.saveProjectIdeas) throw new Error('Idea editing is unavailable on this connection.');
      draft.idea.tags = parseIdeaTags(tags[savingKey] ?? '');
      const ideas = ideaSaveList(project, draft);
      retainedSaving[savingKey] = true; onsaving?.(true); error = '';
      const snapshot = await bridge.saveProjectIdeas(projectId, ideas, draft.revision);
      // Complete the cache write even if navigation already unmounted this instance.
      delete retainedDrafts[savingKey];
      delete retainedTags[savingKey];
      drafts[savingKey] = undefined;
      tags[savingKey] = '';
      onUpdate(snapshot);
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { if (retainedSaving[savingKey]) { delete retainedSaving[savingKey]; onsaving?.(false); } }
  }
</script>

<section class="ideas" aria-label={`${project.name} Idea List`}>
  <header><strong>{project.name} · Ideas</strong><button disabled={saving || hasEdits()} onclick={capture}>Capture idea</button></header>
  <div class="layout">
    <nav aria-label="Ideas">
      {#each project.ideas ?? [] as idea (idea.id)}
        <button disabled={saving} class:active={draft?.idea.id === idea.id} onclick={() => select(idea.id)}><span>{idea.title}</span><small>{idea.status ?? 'open'}{idea.tags?.length ? ` · ${idea.tags.join(', ')}` : ''}</small></button>
      {:else}<p>Capture possible follow-up work here.</p>{/each}
    </nav>
    <main>
      {#if error}<p role="alert" class="error">{error}</p>{/if}
      {#if draft}
        {#if changedElsewhere}<p role="alert" class="error">The list changed elsewhere. Your draft is retained. Copy any changes you need, then discard and reopen the latest idea.</p>{/if}
        <label>Title<input aria-label="Idea title" bind:value={draft.idea.title} maxlength={240} disabled={saving}/></label>
        <div class="fields"><label>Status<select bind:value={draft.idea.status} disabled={saving}>{#each ideaStatuses as status}<option value={status}>{status === 'inPlan' ? 'In plan' : status}</option>{/each}</select></label><label>Tags<input aria-label="Idea tags" placeholder="Comma-separated tags" bind:value={tags[cacheKey]} disabled={saving}/></label></div>
        <div class="actions"><button disabled={saving} onclick={() => preview = !preview}>{preview ? 'Edit Markdown' : 'Preview'}</button><span>By {draft.idea.authorName ?? (draft.idea.authorKind === 'person' ? 'You' : 'Unknown')}</span></div>
        {#if preview}<div class="preview"><Markdown text={draft.idea.markdown}/></div>{:else}<label>Details<textarea aria-label="Idea Markdown" bind:value={draft.idea.markdown} disabled={saving} placeholder="Markdown details"></textarea></label>{/if}
        <div class="actions"><button disabled={saving || changedElsewhere} onclick={save}>{saving ? 'Saving…' : 'Save idea'}</button><button disabled={saving} onclick={() => { drafts[cacheKey] = undefined; error = ''; }}>Discard draft</button></div>
        {#if draft.idea.history?.length}<details><summary>Edit history ({draft.idea.history.length})</summary>{#each [...draft.idea.history].reverse() as edit}<article><small>{new Date(edit.editedAt).toLocaleString()} · {edit.editedBy.name ?? edit.editedBy.kind}</small><strong>{edit.title}</strong><Markdown text={edit.markdown}/></article>{/each}</details>{/if}
      {:else}<p>Select an idea to read or edit its document. Parking an idea does not authorize implementation.</p>{/if}
    </main>
  </div>
</section>

<style>
  .ideas{height:100%;min-height:0;display:flex;flex-direction:column;color:var(--ink);background:var(--panel)}
  header,.actions,.fields{display:flex;gap:10px;align-items:center}header{justify-content:space-between;padding:12px;border-bottom:1px solid var(--line)}
  .layout{display:grid;grid-template-columns:minmax(160px,30%) 1fr;flex:1;min-height:0}nav,main{overflow:auto;padding:12px}nav{border-right:1px solid var(--line)}
  nav button{width:100%;text-align:left;display:flex;flex-direction:column;margin-bottom:6px}small,.actions span{opacity:.65;font-size:12px}.active{outline:1px solid var(--accent)}
  label{display:flex;flex-direction:column;gap:6px;margin-bottom:12px;flex:1}input,textarea,select{width:100%;box-sizing:border-box;padding:8px;color:inherit;background:var(--panel);border:1px solid var(--line);border-radius:6px}textarea{min-height:240px;resize:vertical;font-family:monospace}button{padding:7px 10px;border:1px solid var(--line);border-radius:6px;color:inherit;background:var(--panel)}button:disabled{opacity:.5}.error{color:var(--error-color,#c95454)}.actions{margin-bottom:12px;flex-wrap:wrap}.preview{min-height:180px;overflow-wrap:anywhere}article{padding:10px 0;border-bottom:1px solid var(--line)}article strong{display:block}p{opacity:.8;font-size:13px}
  .ideas small,.actions span{color:var(--muted)}
  @media(max-width:620px){button,input,select{min-height:44px}summary{min-height:44px;padding:8px 0;box-sizing:border-box}.layout{display:flex;flex-direction:column;overflow:auto}nav{max-height:160px;flex-shrink:0;border-right:0;border-bottom:1px solid var(--line)}main{overflow:visible}.fields{flex-direction:column;align-items:stretch}textarea{min-height:200px}}
</style>
