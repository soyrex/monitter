import { mount } from 'svelte';
import Harness from './lazy-rich-markdown-harness.svelte';

mount(Harness, { target: document.getElementById('app') });
