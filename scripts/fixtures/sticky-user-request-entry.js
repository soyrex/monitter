import { mount } from 'svelte';
import Harness from './sticky-user-request-harness.svelte';

mount(Harness, { target: document.querySelector('#app') });
