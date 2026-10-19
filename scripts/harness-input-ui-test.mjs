import { chromium, webkit, expect } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { build } from 'vite';
import { createServer } from 'node:http';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Compile only the real input renderer. This fixture never connects a bridge,
// resolves a permission request, or starts a provider turn.
const root = process.cwd();
mkdirSync(join(root, 'artifacts', 'harness-input-ui'), { recursive: true });
const fixture = mkdtempSync(join(root, 'artifacts', 'harness-input-ui', 'fixture-'));
writeFileSync(join(fixture, 'main.js'), `
import { mount, unmount } from 'svelte';
import HarnessInput from ${JSON.stringify(join(root, 'src/lib/components/HarnessInput.svelte'))};
let instance;
window.renderInput = async (input, props = {}) => {
  if (instance) await unmount(instance);
  window.responses = [];
  instance = mount(HarnessInput, { target: document.querySelector('#app'), props: {
    input, formId: 'fixture-input', ...props,
    onsubmit: value => window.responses.push(structuredClone(value)),
  }});
};
`);
const entry = relative(root, join(fixture, 'main.js'));
let server;
try {
  const result = await build({
    root, configFile: false, cacheDir: join(fixture, 'cache'), plugins: [svelte()],
    resolve: { alias: [{ find: '$lib', replacement: join(root, 'src/lib') }] },
    build: { write: false, minify: false, rollupOptions: { input: join(root, entry) } },
  });
  const files = new Map(result.output.map(file => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]));
  const bundle = result.output.find(file => file.type === 'chunk' && file.isEntry);
  const styles = result.output.filter(file => file.type === 'asset' && file.fileName.endsWith('.css'));
  files.set('/', `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
    <style>:root{--line:#ccc;--panel:#fff;--ink:#222;--muted:#666;--accent:#076d42;--on-accent:white}body{font-family:system-ui}</style>
    ${styles.map(file => `<link rel="stylesheet" href="/${file.fileName}">`).join('')}
    <div id="app"></div><button form="fixture-input" type="submit">External submit</button>
    <script type="module" src="/${bundle.fileName}"></script>`);
  server = createServer((request, response) => {
    const body = files.get(request.url);
    response.writeHead(body === undefined ? 404 : 200, { 'content-type': request.url === '/' ? 'text/html' : request.url.endsWith('.css') ? 'text/css' : 'text/javascript' });
    response.end(body);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const question = (changes = {}) => ({ id: 'q', header: 'Choice', question: 'Which items?', isSecret: false,
    options: [{ label: 'Alpha', description: 'First choice' }, { label: 'Beta', description: 'Second choice' }], ...changes });
  const questions = (...items) => ({ kind: 'questions', questions: items, schema: null, url: null });

  for (const engine of [chromium, webkit]) {
    const browser = await engine.launch(engine === chromium && process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {});
    try {
      for (const mobile of [false, true]) {
        const page = await browser.newPage({ viewport: mobile ? { width: 390, height: 844 } : { width: 1100, height: 850 },
          ...(engine === webkit ? { isMobile: mobile } : {}), hasTouch: mobile });
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.goto(base);
        await page.waitForFunction(() => typeof window.renderInput === 'function');
        const render = async (input, props = {}) => {
          await page.evaluate(({ input, props }) => window.renderInput(input, props), { input, props });
          await expect(page.locator('.harness-input')).toBeVisible();
        };
        // Playwright's object deserializer skips __proto__ keys. Cross this
        // boundary as JSON so the fixture checks the submitted wire shape.
        const responses = async () => JSON.parse(await page.evaluate(() => JSON.stringify(window.responses)));
        const submit = () => page.getByRole('button', { name: 'Submit response', exact: true }).click();

        const draftProps = { draftKey: JSON.stringify(['task', 'pending-question']) };
        await render(questions(question({ selectionMode: 'multiple' })), draftProps);
        await page.getByRole('checkbox', { name: 'Alpha First choice', exact: true }).check();
        await page.getByLabel('Which items?', { exact: true }).fill('Unsubmitted draft');
        await render(questions(question({ id: 'different', question: 'Different task?' })), { draftKey: JSON.stringify(['other-task', 'pending-question']) });
        await expect(page.getByLabel('Different task?', { exact: true })).toHaveValue('');
        await render(questions(question({ selectionMode: 'multiple' })), draftProps);
        await expect(page.getByLabel('Which items?', { exact: true })).toHaveValue('Unsubmitted draft');
        await expect(page.getByRole('checkbox', { name: 'Alpha First choice', exact: true })).toBeChecked();
        expect(await responses()).toEqual([]);
        await render(questions(question({ selectionMode: 'multiple' })));
        expect(await responses()).toEqual([]);
        await submit();
        await expect(page.getByRole('alert')).toHaveText('Please answer each question.');
        expect(await responses()).toEqual([]);
        await page.getByRole('checkbox', { name: 'Beta Second choice', exact: true }).check();
        await page.getByRole('checkbox', { name: 'Alpha First choice', exact: true }).check();
        await page.getByLabel('Which items?', { exact: true }).fill('  Custom answer  ');
        await page.screenshot({ path: join(root, 'artifacts', 'harness-input-ui', `multiple-${engine.name()}-${mobile ? 'mobile' : 'desktop'}.png`), fullPage: true });
        await submit();
        expect(await responses()).toEqual([{ answers: { q: { answers: ['Alpha', 'Beta', 'Custom answer'] } } }]);
        await page.getByRole('checkbox', { name: 'Alpha First choice', exact: true }).uncheck();
        await page.getByLabel('Which items?', { exact: true }).fill('Beta');
        await submit();
        expect((await responses()).at(-1)).toEqual({ answers: { q: { answers: ['Beta'] } } });
        await page.getByRole('checkbox', { name: 'Beta Second choice', exact: true }).uncheck();
        await page.getByLabel('Which items?', { exact: true }).fill('Only free text');
        await submit();
        expect((await responses()).at(-1)).toEqual({ answers: { q: { answers: ['Only free text'] } } });

        await render(questions(question({ id: '__proto__', selectionMode: 'multiple' })));
        await page.getByRole('checkbox', { name: 'Alpha First choice', exact: true }).check();
        await submit();
        expect(await page.evaluate(() => Object.hasOwn(window.responses[0].answers, '__proto__'))).toBe(true);
        expect(await responses()).toEqual([{ answers: { ['__proto__']: { answers: ['Alpha'] } } }]);

        for (const selectionMode of [undefined, 'single']) {
          await render(questions(question({ selectionMode })));
          await page.getByLabel('Choice choices').selectOption('Alpha');
          await submit();
          expect(await responses()).toEqual([{ answers: { q: { answers: ['Alpha'] } } }]);
          await page.getByLabel('Which items?', { exact: true }).fill('Custom single');
          await submit();
          expect((await responses()).at(-1)).toEqual({ answers: { q: { answers: ['Custom single'] } } });
        }

        await render(questions(question({ selectionMode: 'multiple' }), question({ id: 'secret', question: 'Secret value?', options: [], isSecret: true })));
        await page.getByRole('checkbox', { name: 'Alpha First choice', exact: true }).check();
        await submit();
        expect(await responses()).toEqual([]);
        await expect(page.getByLabel('Secret value?', { exact: true })).toHaveAttribute('type', 'password');
        await page.getByLabel('Secret value?', { exact: true }).fill(' private ');
        await submit();
        expect(await responses()).toEqual([{ answers: { q: { answers: ['Alpha'] }, secret: { answers: ['private'] } } }]);

        await render(questions(question({ options: [] })));
        await page.getByLabel('Which items?', { exact: true }).fill('🙂'.repeat(4097));
        await submit();
        await expect(page.getByRole('alert')).toHaveText('Each answer must be 16 KiB or less.');
        expect(await responses()).toEqual([]);
        await page.getByLabel('Which items?', { exact: true }).fill('🙂'.repeat(4096));
        await submit();
        expect((await responses())[0].answers.q.answers).toEqual(['🙂'.repeat(4096)]);

        await render(questions(question({ selectionMode: 'multiple' })), { disabled: true, hideSubmit: true });
        await expect(page.getByRole('checkbox').first()).toBeDisabled();
        await expect(page.getByLabel('Which items?', { exact: true })).toBeDisabled();
        await page.getByRole('button', { name: 'External submit' }).click();
        expect(await responses()).toEqual([]);
        await render(questions(question({ selectionMode: 'multiple' })), { hideSubmit: true });
        await page.getByRole('checkbox', { name: 'Beta Second choice', exact: true }).check();
        await page.getByRole('button', { name: 'External submit' }).click();
        expect(await responses()).toEqual([{ answers: { q: { answers: ['Beta'] } } }]);

        await render({ kind: 'form', questions: [], url: null, schema: { required: ['name'], properties: {
          name: { type: 'string', title: 'Name' }, count: { type: 'integer', title: 'Count' },
          enabled: { type: 'boolean', title: 'Enabled' }, colour: { type: 'string', title: 'Colour', enum: ['Blue', 'Red'] },
        } } });
        await submit();
        expect(await responses()).toEqual([]);
        await page.getByLabel('Name', { exact: true }).fill('Sample');
        await page.getByLabel('Count', { exact: true }).fill('3');
        // Implicit select labels can include option text in their accessible
        // name. Match the field title without requiring an empty select label.
        await page.getByRole('combobox', { name: /^Enabled\b/ }).selectOption('false');
        await page.getByRole('combobox', { name: /^Colour\b/ }).selectOption('Blue');
        await submit();
        expect(await responses()).toEqual([{ name: 'Sample', count: 3, enabled: false, colour: 'Blue' }]);
        await render({ kind: 'form', questions: [], url: null, schema: { properties: { nested: { type: 'object' } } } }, { hideSubmit: true });
        await page.getByRole('button', { name: 'External submit' }).click();
        expect(await responses()).toEqual([]);

        await render({ kind: 'url', questions: [], schema: null, url: 'https://example.com/confirm' });
        expect(await responses()).toEqual([]);
        await expect(page.getByRole('link')).toHaveAttribute('rel', 'noopener noreferrer');
        await page.getByRole('button', { name: 'I have completed this step' }).click();
        expect(await responses()).toEqual([true]);
        await render({ kind: 'url', questions: [], schema: null, url: 'javascript:alert(1)' }, { hideSubmit: true });
        await expect(page.getByRole('link')).toHaveCount(0);
        await page.getByRole('button', { name: 'External submit' }).click();
        expect(await responses()).toEqual([]);
        expect(errors).toEqual([]);
        await page.close();
      }
      console.log(`${engine.name()}: desktop/mobile multi-choice, free text, byte limits and existing input paths passed.`);
    } finally { await browser.close(); }
  }
} finally {
  if (server) await new Promise(resolve => server.close(resolve));
  rmSync(fixture, { recursive: true, force: true });
}
