<script lang="ts">
  import { GitBranch, Play, Terminal } from '@lucide/svelte';
  import type { Task, TaskGitStatus, TerminalSession } from '$lib/types';
  let { task, gitStatus = null, tasks = [], terminalSessions = [], onOpenGit }: { task: Task; gitStatus?: TaskGitStatus | null; tasks?: Task[]; terminalSessions?: TerminalSession[]; onOpenGit?: () => void } = $props();
  const tracked = $derived([
    ...tasks.filter(item => item.status === 'running' && item.hostId === task.hostId && item.cwd === task.cwd).map(item => ({ key:`task:${item.id}`, label: item.id === task.id ? `${item.provider} task` : `${item.provider} task · ${item.title}` })),
    ...terminalSessions.filter(item => item.status === 'running' && item.hostId === task.hostId && item.cwd === task.cwd).map(item => ({ key:`terminal:${item.id}`, label:`Terminal · ${item.title}` })),
  ]);
</script>

<section class="run-summary" aria-label="Run summary">
  {#if gitStatus?.repository}<button class="git" onclick={onOpenGit}><GitBranch size={14}/><span>{gitStatus.branch || 'Detached HEAD'}</span><small>{#if !gitStatus.files.length}Working tree clean{:else}{gitStatus.files.filter(file=>file.indexStatus.trim()&&!file.untracked).length} staged · {gitStatus.files.filter(file=>file.worktreeStatus.trim()&&!file.untracked).length} changed · {gitStatus.files.filter(file=>file.untracked).length} untracked{gitStatus.truncated ? '+' : ''}{/if}</small></button>{/if}
  <div class="processes"><h3>TRACKED PROCESSES</h3>{#each tracked as item (item.key)}<p><Play size={11}/>{item.label}</p>{:else}<p class="empty"><Terminal size={11}/>No Monitter-tracked process in this folder.</p>{/each}</div>
</section>
<style>.run-summary{display:grid;gap:9px;padding:11px;border-bottom:1px solid var(--line);min-width:0}.git{display:grid;grid-template-columns:auto minmax(0,1fr);gap:6px;padding:6px;border:1px solid var(--line);border-radius:5px;text-align:left;min-width:0}.git span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.git small{grid-column:2;color:var(--muted);font-size:10px;white-space:normal}.processes h3{margin:2px 0 5px;font-size:10px;color:var(--muted)}.processes p{display:flex;align-items:center;gap:5px;margin:3px 0;font-size:11px;min-width:0;overflow-wrap:anywhere}.empty{color:var(--muted)}</style>
