import { mount, unmount } from 'svelte';
import AttachmentList from '../../src/lib/components/AttachmentList.svelte';
import { thumbnail, readBrowserFile } from '../../src/lib/attachment-files';
let component;
window.__IMAGE_QA__ = {
  read: async () => { throw new Error('Reader not configured'); },
  thumbnail, readBrowserFile,
  async show(attachments) {
    if (component) await unmount(component);
    component = mount(AttachmentList, { target: document.getElementById('app'), props: { attachments } });
  },
};
