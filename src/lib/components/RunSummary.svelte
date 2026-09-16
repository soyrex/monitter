<script lang="ts">
  import { ChevronRight, GitBranch, Play, SquareTerminal } from '@lucide/svelte';
  import type { Task, TaskGitStatus, TerminalSession } from '$lib/types';
  let { task, gitStatus = null, tasks = [], terminalSessions = [], onOpenGit }: { task: Task; gitStatus?: TaskGitStatus | null; tasks?: Task[]; terminalSessions?: TerminalSession[]; onOpenGit?: () => void } = $props();
  const tracked = $derived([
    ...tasks.filter(item => item.status === 'running' && item.hostId === task.hostId && item.cwd === task.cwd).map(item => ({ key:`task:${item.id}`, label: item.id === task.id ? `${item.provider} task` : `${item.provider} task · ${item.title}` })),
    ...terminalSessions.filter(item => item.status === 'running' && item.hostId === task.hostId && item.cwd === task.cwd).map(item => ({ key:`terminal:${item.id}`, label:`Terminal · ${item.title}` })),
  ]);
</script>

<section class="run-summary" aria-label="Run summary">
  {#if gitStatus?.repository}
    <button class="git" onclick={onOpenGit} title="Open Git changes">
      <span class="git-icon"><GitBranch size={15}/></span>
      <span class="git-body">
        <span class="git-label">Git branch</span>
        <span class="git-branch">{gitStatus.branch || 'Detached HEAD'}</span>
        <span class="git-status">{#if !gitStatus.files.length}Working tree clean{:else}{gitStatus.files.filter(file=>file.indexStatus.trim()&&!file.untracked).length} staged · {gitStatus.files.filter(file=>file.worktreeStatus.trim()&&!file.untracked).length} changed · {gitStatus.files.filter(file=>file.untracked).length} untracked{gitStatus.truncated ? '+' : ''}{/if}</span>
      </span>
      <span class="git-chevron"><ChevronRight size={13}/></span>
    </button>
  {/if}
  <div class="processes"><h3>TRACKED PROCESSES</h3>{#each tracked as item (item.key)}<p><Play size={11}/>{item.label}</p>{:else}<p class="empty"><SquareTerminal size={11}/>No Monitter-tracked process in this folder.</p>{/each}</div>
</section>
<style>.run-summary{display:grid;gap:9px;padding:11px;min-width:0}
.git{display:flex;align-items:center;gap:10px;width:100%;min-width:0;padding:10px 11px;border:1px solid color-mix(in srgb,var(--line) 65%,transparent);border-radius:7px;background:color-mix(in srgb,var(--ink) 3%,var(--sidebar));color:var(--ink);text-align:left;cursor:pointer;transition:background 120ms,border-color 120ms}
.git:hover{background:color-mix(in srgb,var(--accent) 7%,var(--sidebar));border-color:color-mix(in srgb,var(--accent) 35%,var(--line))}
.git:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.git-icon{display:flex;flex:none;align-items:center;justify-content:center;color:var(--accent-ink)}
.git-body{display:flex;flex:1;flex-direction:column;gap:4px;min-width:0}
.git-label{font:500 calc(9px * var(--interface-font-ratio,1))/1.2 var(--mono);letter-spacing:.07em;text-transform:uppercase;color:var(--muted)}
.git-branch{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font:500 calc(12px * var(--interface-font-ratio,1))/1.4 var(--mono)}
.git-status{font:400 calc(10px * var(--interface-font-ratio,1))/1.5 var(--sans, sans-serif);color:var(--muted);white-space:normal;overflow-wrap:anywhere}
.git-chevron{display:flex;flex:none;color:var(--muted)}
.processes{border-top:1px solid var(--line);padding-top:11px}.processes h3{margin:2px 0 5px;font-size:calc(10px * var(--interface-font-ratio, 1));color:var(--muted)}.processes p{display:flex;align-items:center;gap:5px;margin:3px 0;font-size:calc(11px * var(--interface-font-ratio, 1));min-width:0;overflow-wrap:anywhere}.empty{color:var(--muted)}</style>
