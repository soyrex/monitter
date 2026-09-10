import type { Agent } from './types';
export const channelCommands = [
  {id:'channel:members', label:'/members', detail:'Show channel members (/names also works)'},
  {id:'channel:invite', label:'/invite', detail:'Add an agent: /invite @name'},
  {id:'channel:kick', label:'/kick', detail:'Remove an agent and stop its channel run: /kick @name'},
  {id:'channel:topic', label:'/topic', detail:'Show or change the topic: /topic text (use - to clear)'},
  {id:'channel:admin', label:'/admin', detail:'Open channel administration'},
  {id:'channel:help', label:'/help', detail:'Show channel command help'},
];
export function parseChannelCommand(value:string) {
  const match=value.trim().match(/^\/(members|names|invite|kick|topic|admin|help)(?:\s+([\s\S]*))?$/i);
  return match ? {name:match[1].toLowerCase(),args:match[2]?.trim()??''} : null;
}
export function resolveChannelAgent(input:string,agents:Agent[]):Agent {
  const query=input.trim().replace(/^@/,'').replace(/^"(.*)"$/,'$1').toLowerCase();
  if(!query)throw Error('Specify an agent, for example @Rafa.');
  const exact=agents.filter(agent=>[agent.name.toLowerCase(),agent.name.toLowerCase().replace(/\s+/g,'-'),agent.id.toLowerCase()].includes(query));
  const matches=exact.length?exact:agents.filter(agent=>agent.name.trim().split(/\s+/)[0].toLowerCase()===query);
  if(matches.length!==1)throw Error(matches.length?'That name matches several agents. Use the full agent name.':`No agent matches ${input}.`);
  return matches[0];
}
