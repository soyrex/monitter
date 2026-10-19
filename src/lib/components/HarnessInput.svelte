<script lang="ts">
  import { onDestroy } from 'svelte';
  import { readQuestionDraft, saveQuestionDraft } from '$lib/question-drafts';
  import type { InteractionInput } from '$lib/types';
  let { input, disabled = false, onsubmit, formId = undefined, hideSubmit = false, draftKey = undefined }: { input: InteractionInput; disabled?: boolean; onsubmit: (value: unknown) => void; formId?: string; hideSubmit?: boolean; draftKey?: string } = $props();
  const initialDraft = readQuestionDraft(draftKey);
  let values = $state<Record<string, string>>(initialDraft.values);
  let selections = $state<Record<string, string[]>>(initialDraft.selections);
  let loadedDraftKey = draftKey;
  const persistDraft = () => saveQuestionDraft(loadedDraftKey, { values, selections });
  $effect(() => {
    if (loadedDraftKey !== draftKey) {
      const draft = readQuestionDraft(draftKey);
      loadedDraftKey = draftKey;
      values = draft.values;
      selections = draft.selections;
      return;
    }
    persistDraft();
  });
  onDestroy(persistDraft);
  let error = $state('');
  const fields = $derived(Object.entries((input.schema?.properties ?? {}) as Record<string, Record<string, unknown>>));
  const supported = $derived(input.kind !== 'form' || fields.every(([, field]) => ['string', 'number', 'integer', 'boolean'].includes(String(field.type))));
  const safeUrl = $derived.by(() => { try { const url = new URL(input.url ?? ''); return ['https:', 'http:'].includes(url.protocol) ? url.href : null; } catch { return null; } });
  function submit(event: SubmitEvent) {
    event.preventDefault(); error = '';
    // A dock may place the submit control outside this form so it can stay
    // reachable. Keep the same safety gate as the built-in disabled button.
    if (disabled || !supported || (input.kind === 'url' && !safeUrl)) return;
    if (input.kind === 'questions') {
      const answers: Record<string, { answers: string[] }> = Object.create(null);
      for (const question of input.questions) {
        const key = `question:${question.id}`;
        const value = values[key]?.trim();
        const selected = question.selectionMode === 'multiple'
          ? question.options.filter(option => selections[key]?.includes(option.label)).map(option => option.label)
          : [];
        const response = [...new Set([...selected, ...(value ? [value] : [])])];
        if (!response.length) { error = 'Please answer each question.'; return; }
        if (response.some(answer => new TextEncoder().encode(answer).length > 16384)) {
          error = 'Each answer must be 16 KiB or less.'; return;
        }
        answers[question.id] = { answers: response };
      }
      onsubmit({ answers });
    } else if (input.kind === 'form' && supported) {
      const response: Record<string, unknown> = Object.create(null);
      for (const [name, field] of fields) {
        const raw = values[name];
        const required = Array.isArray(input.schema?.required) && input.schema.required.includes(name);
        if (raw === undefined || raw === '') { if (required) { error = `Please provide ${name}.`; return; } continue; }
        response[name] = field.type === 'boolean' ? raw === 'true' : ['number', 'integer'].includes(String(field.type)) ? Number(raw) : raw;
      }
      onsubmit(response);
    } else if (input.kind === 'url') { onsubmit(true); }
  }
</script>

<form class="harness-input" id={formId} onsubmit={submit}>
  {#if input.kind === 'questions'}
    {#each input.questions as question (question.id)}
      {@const key = `question:${question.id}`}
      <fieldset><legend>{question.header ? `${question.header}: ` : ''}{question.question}</legend>
        {#if question.selectionMode === 'multiple' && question.options.length}
          <div class="choices">
            {#each question.options as option}
              <label class="choice">
                <input type="checkbox" disabled={disabled} checked={selections[key]?.includes(option.label) ?? false} onchange={event => {
                  const selected = selections[key] ?? [];
                  selections[key] = event.currentTarget.checked
                    ? [...selected, option.label]
                    : selected.filter(label => label !== option.label);
                }} />
                <span>{option.label}{#if option.description}<small>{option.description}</small>{/if}</span>
              </label>
            {/each}
          </div>
        {:else if question.options.length}<select aria-label={`${question.header || question.id} choices`} disabled={disabled} value={values[key] ?? ''} onchange={event => values[key] = event.currentTarget.value}><option value="">Choose an answer…</option>{#each question.options as option}<option value={option.label}>{option.label}{option.description ? ` — ${option.description}` : ''}</option>{/each}</select>{/if}
        <input aria-label={question.question} type={question.isSecret ? 'password' : 'text'} autocomplete="off" disabled={disabled} bind:value={values[key]} placeholder={question.options.length ? 'Or write your answer' : 'Your answer'} maxlength={16384} />
      </fieldset>
    {/each}
  {:else if input.kind === 'form' && supported}
    {#each fields as [name, field]}
      <label><span>{String(field.title ?? name)}</span>{#if field.description}<small>{String(field.description)}</small>{/if}
        {#if Array.isArray(field.enum)}<select disabled={disabled} bind:value={values[name]}><option value="">Select…</option>{#each field.enum as option}<option value={String(option)}>{String(option)}</option>{/each}</select>
        {:else if field.type === 'boolean'}<select disabled={disabled} bind:value={values[name]}><option value="">Select…</option><option value="true">Yes</option><option value="false">No</option></select>
        {:else}<input type={field.type === 'string' ? 'text' : 'number'} step={field.type === 'integer' ? 1 : 'any'} autocomplete="off" disabled={disabled} bind:value={values[name]} />{/if}
      </label>
    {/each}
  {:else if input.kind === 'url'}
    <p>This tool asks you to complete a separate browser step. Check the address before opening it.</p>
    {#if safeUrl}<a href={safeUrl} target="_blank" rel="noopener noreferrer">{safeUrl}</a>{:else}<p>Unsupported link. Deny this request to continue safely.</p>{/if}
  {:else}<p>This form uses unsupported field types. Deny the request and ask the agent for another way.</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#if !hideSubmit}<button type="submit" disabled={disabled || !supported || (input.kind === 'url' && !safeUrl)}>{input.kind === 'url' ? 'I have completed this step' : 'Submit response'}</button>{/if}
</form>

<style>
  .harness-input { display: grid; gap: 12px; margin-top: 12px; }
  label { display: grid; gap: 6px; font-size: 13px; }
  fieldset { min-width: 0; margin: 0; padding: 0; border: 0; display: grid; gap: 6px; font-size: 13px; }
  legend { padding: 0; margin-bottom: 6px; }
  .choices { display: grid; gap: 4px; }
  .choice { display: flex; align-items: flex-start; gap: 10px; min-height: 40px; padding: 6px 0; box-sizing: border-box; }
  .choice input { flex: none; width: 18px; height: 18px; min-height: 18px; padding: 0; margin: 2px 0; accent-color: var(--accent); }
  .choice small { display: block; margin-top: 2px; }
  small, p { color: var(--muted); }
  input, select { min-width: 0; width: 100%; box-sizing: border-box; min-height: 40px; padding: 8px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); color: var(--ink); font: inherit; }
  button { justify-self: start; min-height: 40px; border: 1px solid var(--accent); border-radius: 6px; padding: 8px 12px; background: var(--accent); color: var(--on-accent); font: inherit; }
  button:disabled { opacity: .5; }
  a { color: var(--accent-ink); overflow-wrap: anywhere; }
  @media (max-width: 760px) { input, select { font-size: 16px; } }
</style>
