import { observeBrowserViewport } from '../../src/lib/browser-viewport';
const node = document.querySelector('#viewport');
const reports = window.reports = [];
const observer = observeBrowserViewport(node, rect => reports.push(rect));
window.geometry = observer;
observer.update(true, false, 0);
