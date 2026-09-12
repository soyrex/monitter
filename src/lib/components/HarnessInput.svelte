<script lang="ts">
  import type { InteractionInput } from '$lib/types';
  let { input, disabled = false, onsubmit, formId = undefined, hideSubmit = false }: { input: InteractionInput; disabled?: boolean; onsubmit: (value: unknown) => void; formId?: string; hideSubmit?: boolean } = $props();
  let values = $state<Record<string, string>>({});
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
      const answers: Record<string, { answers: string[] }> = {};
      for (const question of input.questions) {
        const value = values[question.id]?.trim();
        if (!value) { error = 'Please answer each question.'; return; }
        answers[question.id] = { answers: [value] };
      }
      onsubmit({ answers });
    } else if (input.kind === 'form' && supported) {
      const response: Record<string, unknown> = {};
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
      <label><span>{question.header ? `${question.header}: ` : ''}{question.question}</span>
        {#if question.options.length}<select aria-label={`${question.header || question.id} choices`} disabled={disabled} value={values[question.id] ?? ''} onchange={event => values[question.id] = event.currentTarget.value}><option value="">Choose an answer…</option>{#each question.options as option}<option value={option.label}>{option.label}{option.description ? ` — ${option.description}` : ''}</option>{/each}</select>{/if}
        <input aria-label={question.question} type={question.isSecret ? 'password' : 'text'} autocomplete="off" disabled={disabled} bind:value={values[question.id]} placeholder={question.options.length ? 'Or write your answer' : 'Your answer'} maxlength={16384} />
      </label>
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
  small, p { color: var(--muted); }
  input, select { min-width: 0; width: 100%; box-sizing: border-box; min-height: 40px; padding: 8px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); color: var(--ink); font: inherit; }
  button { justify-self: start; min-height: 40px; border: 1px solid var(--accent); border-radius: 6px; padding: 8px 12px; background: var(--accent); color: var(--on-accent); font: inherit; }
  button:disabled { opacity: .5; }
  a { color: var(--accent-ink); overflow-wrap: anywhere; }
  @media (max-width: 760px) { input, select { font-size: 16px; } }
</style>
