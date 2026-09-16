import { mount } from 'svelte';
import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource/ibm-plex-mono/400.css';
import '@fontsource/ibm-plex-mono/500.css';
import Share from '../src/routes/share/+page.svelte';

// Visitor-only entry point: no owner/LAN bridge or workspace interface.
mount(Share, { target: document.getElementById('app')! });
