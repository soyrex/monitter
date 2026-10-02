const escapeHtml = (value: string) => value.replace(/[&<>"']/g, character => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[character]!);

/** Keep mail markup isolated from the application and prevent network requests. */
export function mailReaderDocument(bodyHtml: string | null | undefined, bodyText: string): string {
  let content: string;
  if (bodyHtml) {
    const document = new DOMParser().parseFromString(bodyHtml, 'text/html');
    document.querySelectorAll('script,iframe,frame,object,embed,form,input,button,textarea,select,link,meta,base,svg,math,video,audio,source').forEach(element => element.remove());
    document.querySelectorAll('*').forEach(element => {
      for (const attribute of [...element.attributes]) {
        if (/^on/i.test(attribute.name) || ['srcset', 'action', 'formaction', 'ping', 'srcdoc', 'background'].includes(attribute.name.toLowerCase())) element.removeAttribute(attribute.name);
      }
      if (element.tagName === 'IMG' && !/^data:image\/(png|gif|jpeg|webp);base64,/i.test(element.getAttribute('src') ?? '')) element.removeAttribute('src');
      if (element.tagName === 'A') { element.removeAttribute('href'); element.removeAttribute('target'); }
    });
    content = document.body.innerHTML;
  } else {
    let depth = 0;
    content = '<div class="plain-message">';
    for (const line of bodyText.split('\n')) {
      const prefix = line.match(/^\s*((?:>\s*)+)/);
      const nextDepth = prefix ? (prefix[1].match(/>/g)?.length ?? 0) : 0;
      while (depth < nextDepth) { content += '<blockquote>'; depth += 1; }
      while (depth > nextDepth) { content += '</blockquote>'; depth -= 1; }
      content += `<div>${escapeHtml(prefix ? line.slice(prefix[0].length) : line) || '<br>'}</div>`;
    }
    content += '</blockquote>'.repeat(depth) + '</div>';
  }
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action 'none'; base-uri 'none'"><style>
    :root { color-scheme:light; } * { box-sizing:border-box; } body { margin:0;padding:28px 32px;color:#25313b;background:#fff;font:14px/1.65 -apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;overflow-wrap:anywhere; } img { max-width:100%;height:auto; } table { max-width:100%; } pre { white-space:pre-wrap; } .plain-message { white-space:pre-wrap; } blockquote,.gmail_quote { margin:14px 0 14px 8px;padding:4px 0 4px 16px;border-left:3px solid #b8cbdc;color:#526477; } blockquote blockquote,.gmail_quote .gmail_quote { border-color:#c8b8dd;color:#665779; } blockquote blockquote blockquote { border-color:#b4cfc1;color:#526d60; } .gmail_attr { margin:16px 0 8px;color:#687789;font-size:12px; } a { color:#2b669a; } hr { border:0;border-top:1px solid #dce2e8;margin:20px 0; } @media(max-width:600px) { body { padding:18px 16px; } }
    </style></head><body>${content}</body></html>`;
}
