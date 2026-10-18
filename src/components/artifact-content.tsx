import { Fragment } from "react";

export function artifactContentUrl(agentId: string, id: string, inline = false) {
  return `/api/agents/${encodeURIComponent(agentId)}/artifacts/${encodeURIComponent(id)}/content${inline ? "?inline=1" : ""}`;
}
/** Intentionally small renderer: escaped text and same-Agent artifact references,
 * never raw HTML, external images, executable Markdown, or recursive reports.
 */
export function ArtifactContent({ agentId, content, attachments = [] }: { agentId: string; content: string; attachments?: { id: string; title: string; mediaType: string }[] }) {
  const matches = [...content.matchAll(/!\[([^\]\n]{0,160})\]\(artifact:([a-f0-9-]{36})\)|artifact:([a-f0-9-]{36})/g)];
  let previous = 0;
  const parts = matches.map((match, index) => {
    const before = content.slice(previous, match.index); previous = match.index + match[0].length;
    const id = match[2] ?? match[3]!;
    const attachment = attachments.find((item) => item.id === id);
    return <Fragment key={index}>{before}{match[2] && attachment?.mediaType.startsWith("image/") ? <img loading="lazy" className="my-3 max-h-[36rem] max-w-full rounded-lg object-contain" src={artifactContentUrl(agentId, id, true)} alt={match[1] || attachment.title} /> : <a className="cursor-pointer text-violet-300 underline" href={`/agents/${encodeURIComponent(agentId)}/artifacts?artifact=${encodeURIComponent(id)}`}>{attachment?.title ?? `Artifact ${id}`}</a>}</Fragment>;
  });
  return <div className="whitespace-pre-wrap break-words text-sm text-zinc-300">{parts}{content.slice(previous)}</div>;
}
