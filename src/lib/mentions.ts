export type MentionAgent = { id: string; name: string };
export type MentionPart = { text: string; agentId?: string };

// Only unique aliases resolve. Email addresses and partial/longer names stay plain text.
export function mentionParts(text: string, agents: MentionAgent[]): MentionPart[] {
  const aliases = new Map<string, Set<string>>();
  for (const agent of agents) {
    const name = agent.name.trim().toLowerCase();
    for (const alias of new Set([name, name.replace(/\s+/g, '-'), name.split(/\s+/)[0]])) {
      if (!alias) continue;
      const ids = aliases.get(alias) ?? new Set<string>(); ids.add(agent.id); aliases.set(alias, ids);
    }
  }
  const names = [...aliases.keys()].filter(name => aliases.get(name)!.size === 1).sort((a,b)=>b.length-a.length);
  if (!names.length) return [{text}];
  const escape = (name: string) => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(`(^|[^\\p{L}\\p{N}_@])@(${names.map(escape).join('|')})(?![\\p{L}\\p{N}_@-])`, 'giu');
  const parts: MentionPart[] = []; let cursor=0;
  for (const match of text.matchAll(pattern)) {
    const start=match.index!+match[1].length, end=match.index!+match[0].length;
    if(start>cursor) parts.push({text:text.slice(cursor,start)});
    parts.push({text:text.slice(start,end),agentId:[...aliases.get(match[2].toLowerCase())!][0]}); cursor=end;
  }
  if(cursor<text.length)parts.push({text:text.slice(cursor)});
  return parts;
}
export function mentionedAgentIds(text: string, agents: MentionAgent[]): string[] {
  return [...new Set(mentionParts(text,agents).flatMap(part=>part.agentId?[part.agentId]:[]))];
}
