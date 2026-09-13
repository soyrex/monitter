<script lang="ts">
  import { onDestroy, tick, untrack } from "svelte";
  import { Archive, Bot, Brain, Columns2, Command, CornerDownLeft, Folder, Grid2X2, MessageSquare, Monitor, Moon, MousePointer2, PanelRight, Radio, Search, Settings2, Sparkles, Square, SquareTerminal, Sun, Users, Wrench, X, ZoomIn, ZoomOut } from "@lucide/svelte";
  import { animateMotion } from "$lib/motion";

  function itemIcon(id: string) {
    const kind = id.split(':')[0];
    if (kind === 'task' || kind === 'draft' || id === 'new-task') return MessageSquare;
    if (kind === 'terminal' || id === 'new-terminal' || id === 'vim-command') return SquareTerminal;
    if (kind === 'channel' || id === 'new-channel') return Radio;
    if (kind === 'agent' || id === 'new-agent' || id === 'steer-busy') return Bot;
    if (kind === 'project' || id === 'new-project' || id === 'sidebar:projects') return Folder;
    if (kind === 'settings' || id === 'appearance') return Settings2;
    if (id === 'agent-directory') return Users;
    if (id === 'archive' || id === 'archived') return Archive;
    if (id === 'reasoning') return Brain;
    if (id === 'tools') return Wrench;
    if (id === 'autoname') return Sparkles;
    if (id === 'stop' || id === 'layout:single') return Square;
    if (id === 'layout:columns') return Columns2;
    if (id === 'layout:grid') return Grid2X2;
    if (id === 'focus-mouse') return MousePointer2;
    if (id === 'enter') return CornerDownLeft;
    if (id === 'scale-up') return ZoomIn;
    if (id === 'scale-down') return ZoomOut;
    if (id === 'theme:light') return Sun;
    if (id === 'theme:dark') return Moon;
    if (id === 'hosts' || id === 'theme:system' || id === 'scale-reset') return Monitor;
    if (kind === 'sidebar' || id === 'detail' || id === 'dim-panes') return PanelRight;
    return Command;
  }

  export type CommandPaletteItem = {
    id: string;
    label: string;
    detail?: string;
    group?: string;
    keywords?: string;
    checked?: boolean;
    disabled?: boolean;
  };

  let {
    open = false,
    title,
    placeholder,
    items,
    onselect,
    onclose,
  }: {
    open?: boolean;
    title: string;
    placeholder: string;
    items: CommandPaletteItem[];
    onselect: (id: string) => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let activeIndex = $state(0);
  let input = $state<HTMLInputElement>();
  let dialog = $state<HTMLElement>();
  let results = $state<HTMLDivElement>();
  let previouslyFocused: HTMLElement | null = null;
  let visible = $state(false);
  let closing = $state(false);
  let closeRequested = false;
  let lifecycle = 0;
  let animations: Animation[] = [];
  let focusedWhenClosing = false;

  const filteredItems = $derived.by(() => {
    const normalizedQuery = query.toLocaleLowerCase().trim();
    const terms = normalizedQuery.split(/\s+/).filter(Boolean);

    return items
      .map((item, index) => ({ item, index, score: matchScore(item, normalizedQuery, terms) }))
      .filter((match) => match.score !== undefined)
      .sort((left, right) => left.score! - right.score! || left.index - right.index)
      .map((match) => match.item);
  });

  const groupedItems = $derived.by(() => {
    const groups = new Map<string, CommandPaletteItem[]>();
    for (const item of filteredItems) {
      const group = item.group ?? "Commands";
      groups.set(group, [...(groups.get(group) ?? []), item]);
    }
    return [...groups];
  });
  const selectableItems = $derived(
    groupedItems.flatMap(([, groupItems]) => groupItems).filter((item) => !item.disabled),
  );

  $effect(() => {
    query;
    items;
    activeIndex = selectableItems.length ? 0 : -1;
  });

  function cancelAnimations() {
    for (const animation of animations) animation.cancel();
    animations = [];
  }

  function requestClose() {
    if (!open || closeRequested) return;
    closeRequested = true;
    onclose();
    void tick().then(() => {
      if (open) closeRequested = false;
    });
  }

  function finishClose(token: number) {
    if (token !== lifecycle || open) return;
    const activeElement = document.activeElement;
    const shouldRestoreFocus = focusedWhenClosing && (
      !activeElement || activeElement === document.body || activeElement === document.documentElement || dialog?.contains(activeElement)
    );
    visible = false;
    closing = false;
    focusedWhenClosing = false;
    const focusTarget = previouslyFocused;
    previouslyFocused = null;
    if (shouldRestoreFocus) {
      void tick().then(() => {
        const activeElement = document.activeElement;
        const focusStillUnclaimed = !activeElement || activeElement === document.body || activeElement === document.documentElement || dialog?.contains(activeElement);
        if (token === lifecycle && !open && focusStillUnclaimed) focusTarget?.focus();
      });
    }
  }

  function enter() {
    const token = ++lifecycle;
    cancelAnimations();
    closing = false;
    closeRequested = false;
    if (!visible) visible = true;
    if (!previouslyFocused) previouslyFocused = document.activeElement as HTMLElement | null;
    query = "";
    activeIndex = 0;
    void tick().then(() => {
      if (token !== lifecycle || !open || !dialog) return;
      input?.focus();
      const backdrop = dialog.parentElement;
      animations = [
        ...(backdrop ? [animateMotion(backdrop, [{ opacity: 0 }, { opacity: 1 }], { duration: 170, easing: "ease-out", fill: "both" })] : []),
        animateMotion(dialog, [{ opacity: 0, transform: "translateY(6px)" }, { opacity: 1, transform: "translateY(0)" }], { duration: 170, easing: "cubic-bezier(.2,.8,.2,1)", fill: "both" }),
      ].filter((animation): animation is Animation => animation !== null);
    });
  }

  function exit() {
    if (!visible) return;
    const token = ++lifecycle;
    cancelAnimations();
    focusedWhenClosing = !!dialog?.contains(document.activeElement);
    closing = true;
    if (!dialog) {
      finishClose(token);
      return;
    }
    const backdrop = dialog.parentElement;
    animations = [
      ...(backdrop ? [animateMotion(backdrop, [{ opacity: 1 }, { opacity: 0 }], { duration: 110, easing: "ease-in", fill: "both" })] : []),
      animateMotion(dialog, [{ opacity: 1, transform: "translateY(0)" }, { opacity: 0, transform: "translateY(6px)" }], { duration: 110, easing: "ease-in", fill: "both" }),
    ].filter((animation): animation is Animation => animation !== null);
    if (!animations.length) {
      finishClose(token);
      return;
    }
    void Promise.all(animations.map((animation) => animation.finished.catch(() => undefined))).then(() => finishClose(token));
  }

  $effect(() => {
    if (open) untrack(enter);
    else untrack(exit);
  });

  onDestroy(() => {
    ++lifecycle;
    const restore = dialog?.contains(document.activeElement);
    cancelAnimations();
    if (restore) previouslyFocused?.focus();
  });

  $effect(() => {
    if (!visible || activeIndex < 0) return;
    void tick().then(() =>
      results?.querySelector<HTMLElement>("[data-active='true']")?.scrollIntoView({
        block: "nearest",
      }),
    );
  });

  function fuzzyMatch(value: string, term: string) {
    let position = 0;
    for (const character of term) {
      position = value.indexOf(character, position);
      if (position === -1) return false;
      position += 1;
    }
    return true;
  }

  function matchScore(item: CommandPaletteItem, normalizedQuery: string, terms: string[]) {
    if (!terms.length) return 0;

    const label = item.label.toLocaleLowerCase();
    const descriptor = [item.detail, item.keywords, item.group]
      .filter(Boolean)
      .join(" ")
      .toLocaleLowerCase();
    const haystack = `${label} ${descriptor}`;

    if (label === normalizedQuery) return 0;
    if (label.startsWith(normalizedQuery)) return 1;
    if (label.includes(normalizedQuery)) return 2;
    if (terms.every((term) => label.includes(term))) return 3;
    if (descriptor.includes(normalizedQuery) || terms.every((term) => descriptor.includes(term))) return 4;
    return terms.every((term) => fuzzyMatch(haystack, term)) ? 5 : undefined;
  }

  function select(item: CommandPaletteItem | undefined) {
    if (item && !item.disabled) onselect(item.id);
  }

  function moveActive(delta: number) {
    if (!selectableItems.length) return;
    activeIndex = (activeIndex + delta + selectableItems.length) % selectableItems.length;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!visible || closing || !dialog?.contains(event.target as Node)) return;
    if (event.key === "Escape") {
      event.preventDefault();
      requestClose();
    } else if (event.key === "Tab") {
      containFocus(event);
    } else if (event.target === input && event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
    } else if (event.target === input && event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
    } else if (event.target === input && event.key === "Enter") {
      event.preventDefault();
      select(selectableItems[activeIndex]);
    }
  }

  function containFocus(event: KeyboardEvent) {
    if (!dialog) return;
    const focusable = [
      ...dialog.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
      ),
    ].filter((element) => element.getClientRects().length > 0 && !element.closest('[inert]'));
    if (!focusable.length) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    event.preventDefault();
    const current = focusable.indexOf(document.activeElement as HTMLElement);
    const next = current < 0
      ? (event.shiftKey ? focusable.at(-1)! : focusable[0])
      : focusable[(current + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length];
    next.focus();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if visible}
  <div
    class="backdrop"
    class:closing
    role="presentation"
    onclick={(event) => event.currentTarget === event.target && requestClose()}
  >
    <dialog bind:this={dialog} class="palette" inert={closing} data-motion-closing={closing ? 'true' : undefined} open aria-modal="true" aria-label={title} tabindex="-1">
      <header>
        <span class="title"><Command size={15} strokeWidth={2.1} /> {title}</span>
        <button class="close" type="button" aria-label="Close command palette" onclick={requestClose}>
          <X size={16} />
        </button>
      </header>

      <label class="search">
        <Search size={17} aria-hidden="true" />
        <span class="sr-only">Search {title}</span>
        <input bind:this={input} bind:value={query} {placeholder} autocomplete="off" />
        <kbd>esc</kbd>
      </label>

      <div bind:this={results} class="results" aria-label={`${title} results`}>
        {#if groupedItems.length}
          {#each groupedItems as [group, groupItems] (group)}
            <section class="group" aria-label={group}>
              <h2>{group}</h2>
              {#each groupItems as item (item.id)}
                {@const isActive = selectableItems[activeIndex]?.id === item.id}
                {@const ItemIcon = itemIcon(item.id)}
                <button
                  class:active={isActive}
                  class="result"
                  type="button"
                  disabled={item.disabled}
                  data-active={isActive}
                  aria-label={item.checked === undefined ? undefined : `${item.label}, ${item.checked ? "on" : "off"}`}
                  onclick={() => select(item)}
                  onfocus={() => {
                    const nextIndex = selectableItems.findIndex((candidate) => candidate.id === item.id);
                    if (nextIndex >= 0) activeIndex = nextIndex;
                  }}
                >
                  <span class="result-icon" aria-hidden="true"><ItemIcon size={18}/></span>
                  <span class="copy">
                    <span class="label">{item.label}</span>
                    {#if item.detail}<span class="detail">{item.detail}</span>{/if}
                  </span>
                  {#if item.checked !== undefined}
                    <span class:checked={item.checked} class="switch" aria-hidden="true"><span></span></span>
                  {:else}<span class="enter" aria-hidden="true">↵</span>{/if}
                </button>
              {/each}
            </section>
          {/each}
        {:else}
          <p class="empty">No matching commands.</p>
        {/if}
      </div>

      <footer><span><kbd>↑</kbd><kbd>↓</kbd> navigate</span><span><kbd>↵</kbd> select</span></footer>
    </dialog>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; padding: 20px; background: rgba(0, 0, 0, .3); backdrop-filter: blur(3px); -webkit-backdrop-filter: blur(3px); }
  .palette { position: relative; inset: auto; box-sizing: border-box; display: grid; grid-template-rows: auto auto minmax(0, 1fr) auto; width: min(620px, 100%); height: min(530px, calc(100vh - 40px)); min-height: min(340px, calc(100vh - 40px)); margin: 0; padding: 0; overflow: hidden; border: 1px solid var(--line); border-radius: 13px; color: var(--ink); background: var(--panel); box-shadow: 0 12px 40px rgba(0, 0, 0, 0.8) !important; }
  header, footer { display: flex; align-items: center; justify-content: space-between; padding: 12px 14px; border-color: var(--line); background: var(--panel); }
  header { border-bottom: 1px solid var(--line); }
  .title { display: flex; align-items: center; gap: 7px; font: 600 calc(11px * var(--interface-font-ratio, 1)) var(--mono); color: var(--muted); letter-spacing: .02em; text-transform: uppercase; }
  .close { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 5px; color: var(--muted); }
  .close:hover { color: var(--ink); background: var(--soft); }
  .search { display: flex; align-items: center; gap: 10px; padding: 12px 14px; border-bottom: 1px solid var(--line); color: var(--muted); }
  input { min-width: 0; flex: 1; padding: 0; border: 0; outline: 0; color: var(--ink); background: transparent; font-size: calc(15px * var(--interface-font-ratio, 1)); }
  input::placeholder { color: var(--muted); }
  kbd { padding: 2px 5px; border: 1px solid var(--line); border-bottom-width: 2px; border-radius: 4px; color: var(--muted); background: var(--soft); font: calc(9px * var(--interface-font-ratio, 1)) var(--mono); }
  .results { min-height: 0; overflow: auto; overscroll-behavior: contain; padding: 7px; }
  .group + .group { margin-top: 6px; padding-top: 7px; border-top: 1px solid var(--line); }
  h2 { margin: 4px 7px 5px; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); font-weight: 500; letter-spacing: .03em; text-transform: uppercase; }
  .result { display: flex; align-items: center; width: 100%; min-height: 45px; gap: 12px; padding: 8px 9px; border-radius: 7px; text-align: left; }
  .result:hover, .result.active { background: color-mix(in srgb, var(--accent) 11%, var(--soft)); }
  .result.active { box-shadow: inset 2px 0 var(--accent); }
  .result-icon { display:grid; place-items:center; flex:0 0 22px; width:22px; height:22px; color:var(--muted); }
  .result.active .result-icon { color:var(--accent-ink); }
  .copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: calc(13px * var(--interface-font-ratio, 1)); font-weight: 500; }
  .detail { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); font-size: calc(11px * var(--interface-font-ratio, 1)); }
  .enter { color: var(--muted); font: calc(13px * var(--interface-font-ratio, 1)) var(--mono); }
  .switch { display: flex; align-items: center; width: 28px; height: 16px; padding: 2px; border-radius: 99px; background: var(--line); transition: background .15s ease; }
  .switch span { width: 12px; height: 12px; border-radius: 50%; background: var(--panel); box-shadow: 0 1px 2px rgba(0, 0, 0, .2); transition: transform .15s ease; }
  .switch.checked { background: var(--accent); }
  .switch.checked span { transform: translateX(12px); }
  .empty { margin: 30px 0; text-align: center; color: var(--muted); font-size: calc(12px * var(--interface-font-ratio, 1)); }
  footer { justify-content: flex-end; gap: 15px; border-top: 1px solid var(--line); color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  footer span { display: inline-flex; align-items: center; gap: 3px; }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  @media (max-width: 512px) { .backdrop { padding: 12px; } .palette { width: 100%; max-width: 100%; height: min(480px, calc(100vh - 24px)); min-height: 0; } footer { gap: 9px; padding-inline: 10px; } }
</style>
