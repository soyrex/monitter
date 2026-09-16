export function getBridge() { return { readAttachmentImage: (id: string) => (window as any).__IMAGE_QA__.read(id) }; }
