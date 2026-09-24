<script lang="ts" generics="T extends { id: string }">
  import { Check, ChevronDown, Search, X } from '@lucide/svelte';
  import { tick } from 'svelte';
  import { floating } from '$lib/floating';

  export type DraftPickerOption = {
    id: string;
    label: string;
    description?: string;
    /** Optional secondary text shown muted after the label (e.g. provider name). */
    secondary?: string;
    /** Optional swatch color for an icon dot. */
    color?: string | null;
    disabled?: boolean;
  };

  let {
    options,
    value = $bindable(''),
    placeholder,
    searchPlaceholder,
    ariaLabel,
    disabled = false,
    emptyText = 'No matches',
    onchange,
    onselect,
    onopen
  }: {
    options: T[];
    value?: string;
    placeholder?: string;
    searchPlaceholder?: string;
    ariaLabel: string;
    disabled?: boolean;
    emptyText?: string;
    onchange?: (id: string) => void;
    onselect?: (id: string) => void;
    onopen?: () => void;
  } = $props();

  let open = $state(false);
  let query = $state('');
  let anchor = $state<HTMLButtonElement>();
  let root = $state<HTMLDivElement>();
  let panel = $state<HTMLDivElement>();
  let input = $state<HTMLInputElement>();
  let activeIndex = $state(0);
  let id = $props.id();

  const normalizedOptions = $derived(
    options.map((option) => {
      const ext = option as unknown as DraftPickerOption;
      return {
        id: option.id,
        label: ext.label ?? (option as { name?: string }).name ?? option.id,
        description: ext.description,
        secondary: ext.secondary,
        color: ext.color ?? null,
        disabled: ext.disabled ?? false
      };
    })
  );

  const visible = $derived(
    normalizedOptions.filter((entry) => {
      const needle = query.trim().toLowerCase();
      if (!needle) return true;
      const haystack = `${entry.label} ${entry.description ?? ''} ${entry.secondary ?? ''}`.toLowerCase();
      return haystack.includes(needle);
    })
  );

  const selected = $derived(normalizedOptions.find((entry) => entry.id === value) ?? null);
  const selectedLabel = $derived(selected?.label ?? placeholder ?? 'Choose…');
  const showSearch = $derived(normalizedOptions.length > 5);

  function focusPicker() {
    open = true;
    query = '';
    activeIndex = 0;
    onopen?.();
    void tick().then(() => input?.focus());
  }

  export function focus() {
    focusPicker();
  }

  export function focusTrigger() {
    anchor?.focus();
  }

  function close(returnFocus = true) {
    open = false;
    query = '';
    if (returnFocus) anchor?.focus();
  }

  function pick(entry: { id: string; disabled?: boolean }) {
    if (entry.disabled) return;
    value = entry.id;
    open = false;
    query = '';
    onchange?.(entry.id);
    onselect?.(entry.id);
  }

  function move(delta: number) {
    if (!visible.length) return;
    const len = visible.length;
    activeIndex = ((activeIndex + delta) % len + len) % len;
    void tick().then(() => {
      panel?.querySelector<HTMLElement>(`[data-active='true']`)?.scrollIntoView({ block: 'nearest' });
    });
  }

  function onTriggerKey(event: KeyboardEvent) {
    if (disabled) return;
    if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      focusPicker();
    } else if (event.key === 'Escape' && open) {
      event.preventDefault();
      close(false);
    }
  }

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close(false);
      return;
    }
    if (event.key === 'Tab') {
      // Let the panel trap focus naturally — keep the search input focused.
      if (panel) {
        event.preventDefault();
        const focusables = Array.from(
          panel.querySelectorAll<HTMLElement>('button:not(:disabled)')
        ).filter((node) => node.getClientRects().length > 0);
        if (focusables.length) {
          const next = event.shiftKey ? focusables.at(-1)! : focusables[0];
          next.focus();
        } else {
          input?.focus();
        }
      }
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      move(1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      move(-1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const entry = visible[activeIndex];
      if (entry) pick(entry);
    }
  }

  function outside(event: PointerEvent) {
    if (!open) return;
    if (event.target instanceof Node && !root?.contains(event.target)) close(false);
  }

  // Reset active row when the filtered list changes.
  $effect(() => {
    if (!open) return;
    if (activeIndex >= visible.length) activeIndex = Math.max(0, visible.length - 1);
  });
</script>

<svelte:window onpointerdown={outside} />

<div class="draft-picker" bind:this={root}>
  <button
    bind:this={anchor}
    type="button"
    class="draft-trigger"
    class:open
    aria-label={`${ariaLabel}: ${selectedLabel}`}
    aria-haspopup="dialog"
    aria-expanded={open}
    title={selectedLabel}
    {disabled}
    onclick={() => (open ? close(false) : focusPicker())}
    onkeydown={onTriggerKey}
  >
    {#if selected?.color}<span class="dot" style:background={selected.color}></span>{/if}
    <span class="label">{selectedLabel}</span>
    <ChevronDown size={12} />
  </button>
  {#if open && anchor}
    <div
      bind:this={panel}
      class="draft-menu"
      role="dialog"
      aria-label={ariaLabel}
      tabindex="-1"
      use:floating={{ anchor, side: 'above' }}
    >
      {#if showSearch}
        <div class="search-row">
          <Search size={13} />
          <input
            bind:this={input}
            id={`draft-search-${id}`}
            type="search"
            aria-label={`Search ${ariaLabel.toLowerCase()}`}
            placeholder={searchPlaceholder ?? `Search…`}
            bind:value={query}
            onkeydown={onSearchKey}
          />
          <button
            type="button"
            class="close"
            aria-label="Close picker"
            onclick={() => close(false)}
          ><X size={13} /></button>
        </div>
      {/if}
      <div class="draft-list" role="listbox" aria-label={ariaLabel}>
        {#each visible as entry (entry.id)}
          <button
            type="button"
            class="draft-row"
            class:chosen={value === entry.id}
            class:active={activeIndex === visible.indexOf(entry)}
            data-active={activeIndex === visible.indexOf(entry)}
            role="option"
            aria-selected={value === entry.id}
            disabled={entry.disabled}
            onclick={() => pick(entry)}
            onmouseenter={() => (activeIndex = visible.indexOf(entry))}
          >
            {#if entry.color}<span class="dot" style:background={entry.color}></span>{/if}
            <span class="row-copy">
              <b>{entry.label}</b>
              {#if entry.description}<small>{entry.description}</small>{/if}
            </span>
            {#if entry.secondary}<small class="secondary">{entry.secondary}</small>{/if}
            {#if value === entry.id}<Check size={14} />{/if}
          </button>
        {:else}
          <p class="empty">{emptyText}</p>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .draft-picker { position: relative; min-width: 0; }
  .draft-trigger {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 11px;
    border: 1px solid var(--line);
    border-radius: 7px;
    color: var(--ink);
    background: var(--panel);
    font: inherit;
    font-size: calc(12px * var(--interface-font-ratio, 1));
    text-align: left;
    cursor: pointer;
    transition: border-color 120ms ease;
  }
  .draft-trigger:hover:not(:disabled),
  .draft-trigger.open {
    border-color: var(--accent);
  }
  .draft-trigger:disabled {
    color: var(--muted);
    cursor: not-allowed;
  }
  .draft-trigger .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .draft-trigger .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .draft-menu {
    position: fixed;
    inset: auto;
    margin: 0;
    padding: 10px;
    width: min(360px, 90vw);
    max-height: min(420px, 70vh);
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--panel);
    color: var(--ink);
    box-shadow: 0 12px 40px #0004;
    font-size: calc(12px * var(--interface-font-ratio, 1));
  }
  .search-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 8px;
    border-bottom: 1px solid var(--line);
    margin-bottom: 6px;
  }
  .search-row :global(svg) { color: var(--muted); flex: none; }
  .search-row input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    padding: 4px 0;
  }
  .search-row .close {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 5px;
    color: var(--muted);
  }
  .search-row .close:hover { background: var(--soft); color: var(--ink); }
  .draft-list { display: grid; gap: 2px; }
  .draft-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 9px;
    border-radius: 7px;
    text-align: left;
    color: inherit;
    background: transparent;
    border: 0;
    font: inherit;
    cursor: pointer;
  }
  .draft-row.active,
  .draft-row:hover { background: var(--soft); }
  .draft-row.chosen { background: var(--soft); }
  .draft-row[disabled] { opacity: 0.5; cursor: not-allowed; }
  .draft-row .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .draft-row .row-copy { display: grid; gap: 2px; flex: 1; min-width: 0; }
  .draft-row b {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .draft-row small {
    display: block;
    color: var(--muted);
    line-height: 1.35;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .draft-row .secondary { color: var(--muted); flex: none; }
  .draft-row :global(svg) { color: var(--accent-ink); flex: none; }
  .empty {
    margin: 8px 4px;
    color: var(--muted);
    font-size: calc(11px * var(--interface-font-ratio, 1));
  }
  @container (width < 520px) {
    .draft-trigger { padding: 9px 10px; }
  }
</style>
