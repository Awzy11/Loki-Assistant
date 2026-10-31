import { useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { canAlwaysAllow, grantHost, type Block, type PermissionBlock, type ToolBlock } from "./transport";

/** One-line description of a tool call's arguments for the collapsed card header: the first
 * string-valued argument that looks like what the tool is acting on. */
function summarizeInput(input: unknown): string {
  if (!input || typeof input !== "object") return "";
  const obj = input as Record<string, unknown>;
  // Remote commands: the host matters as much as the command.
  if (typeof obj.host === "string" && typeof obj.command === "string") {
    return `${obj.host} $ ${obj.command}`.replace(/\s+/g, " ").slice(0, 160);
  }
  const preferred = ["file_path", "path", "filepath", "command", "query", "pattern", "url", "title", "content"];
  for (const key of preferred) {
    if (typeof obj[key] === "string") return String(obj[key]).replace(/\s+/g, " ").slice(0, 120);
  }
  const first = Object.values(obj).find((v) => typeof v === "string");
  return typeof first === "string" ? first.replace(/\s+/g, " ").slice(0, 120) : "";
}

/** Claude Code names MCP tools `mcp__server__tool`; show `server · tool`. */
function prettyToolName(name: string): string {
  const m = /^mcp__(.+?)__(.+)$/.exec(name);
  return m ? `${m[1]} · ${m[2]}` : name;
}

const RESULT_PREVIEW_CHARS = 4000;

export function ToolCard({ tool }: { tool: ToolBlock }) {
  const [open, setOpen] = useState(false);
  const running = tool.result === undefined;
  const state = running ? "running" : tool.isError ? "error" : "ok";
  const summary = summarizeInput(tool.input);
  const result = tool.result ?? "";
  return (
    <div className={`loki-tool loki-tool-${state}`}>
      <button type="button" className="loki-tool-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <span className="loki-tool-status" aria-label={state}>
          {running ? <span className="loki-spinner" /> : tool.isError ? "✗" : "✓"}
        </span>
        <span className="loki-tool-name">{prettyToolName(tool.name)}</span>
        {summary && <span className="loki-tool-summary">{summary}</span>}
        <span className="loki-tool-chevron">{open ? "▾" : "▸"}</span>
      </button>
      {open && (
        <div className="loki-tool-body">
          <div className="loki-tool-label">Input</div>
          <pre>{JSON.stringify(tool.input, null, 2)}</pre>
          {!running && (
            <>
              <div className="loki-tool-label">{tool.isError ? "Error" : "Result"}</div>
              <pre>
                {result.slice(0, RESULT_PREVIEW_CHARS)}
                {result.length > RESULT_PREVIEW_CHARS ? `\n… ${result.length - RESULT_PREVIEW_CHARS} more characters` : ""}
              </pre>
            </>
          )}
        </div>
      )}
    </div>
  );
}

export function PermissionCard({
  block,
  onAnswer,
}: {
  block: PermissionBlock;
  onAnswer?: (id: string, allow: boolean, always: boolean) => void;
}) {
  const summary = summarizeInput(block.input);
  return (
    <div className={`loki-perm loki-perm-${block.status}`}>
      <div className="loki-perm-head">
        <span className="loki-perm-icon">{block.status === "pending" ? "⚠" : block.status === "allowed" ? "✓" : block.status === "resolved" ? "↪" : "✗"}</span>
        <span className="loki-tool-name">{prettyToolName(block.tool)}</span>
        <span className="loki-perm-state">
          {block.status === "pending"
            ? "needs your approval"
            : block.status === "allowed"
              ? block.auto
                ? "allowed (always rule)"
                : "allowed"
              : block.status === "resolved"
                ? "answered on another device"
                : "denied"}
        </span>
      </div>
      {summary && <div className="loki-tool-summary loki-perm-summary">{summary}</div>}
      <details className="loki-perm-details">
        <summary>details</summary>
        <pre>{JSON.stringify(block.input, null, 2)}</pre>
      </details>
      {block.status === "pending" && onAnswer && (
        <div className="loki-perm-actions">
          <button type="button" className="loki-perm-yes" onClick={() => onAnswer(block.id, true, false)}>
            Yes
          </button>
          {block.tool.endsWith("remote_run_command") && typeof (block.input as { host?: unknown })?.host === "string" && (
            <button
              type="button"
              title="Stop asking for non-destructive commands on this host for 2 hours"
              onClick={() => grantHost((block.input as { host: string }).host, 120).then(() => onAnswer(block.id, true, false)).catch(() => onAnswer(block.id, true, false))}
            >
              Yes, trust host 2h
            </button>
          )}
          {canAlwaysAllow(block.tool) && (
            <button type="button" onClick={() => onAnswer(block.id, true, true)} title="Stop asking for this kind of action">
              Yes, always
            </button>
          )}
          <button type="button" className="loki-perm-no" onClick={() => onAnswer(block.id, false, false)}>
            No
          </button>
        </div>
      )}
    </div>
  );
}

export function Markdown({ text }: { text: string }) {
  return (
    <div className="loki-turn-content">
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
    </div>
  );
}

export function BlockList({
  blocks,
  onAnswer,
}: {
  blocks: Block[];
  onAnswer?: (id: string, allow: boolean, always: boolean) => void;
}) {
  return (
    <>
      {blocks.map((b, i) =>
        b.kind === "text" ? (
          <Markdown key={i} text={b.text} />
        ) : b.kind === "permission" ? (
          <PermissionCard key={b.id} block={b} onAnswer={onAnswer} />
        ) : (
          <ToolCard key={b.id || i} tool={b} />
        ),
      )}
    </>
  );
}
