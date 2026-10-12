// A minimal MCP client over streamable HTTP (JSON-RPC via fetch), dependency-free, for
// scripts/browser-check.mjs's agent step. Also usable on its own:
//
//   node scripts/mcp-client.mjs '<printed browser URL with #token=...>' <tool> '<json args>'
//
// It only talks to the URL it is given (openpaperdesign-serve's /mcp) with the token from
// that URL's fragment.

export class McpClient {
  constructor(pageUrl) {
    const u = new URL(pageUrl);
    this.token = new URLSearchParams(u.hash.slice(1)).get('token');
    if (!this.token) throw new Error('the URL has no #token=');
    this.endpoint = `${u.origin}/mcp`;
    this.session = null;
    this.nextId = 1;
  }

  headers() {
    const h = {
      Authorization: `Bearer ${this.token}`,
      'Content-Type': 'application/json',
      Accept: 'application/json, text/event-stream',
    };
    if (this.session) h['mcp-session-id'] = this.session;
    return h;
  }

  async post(message) {
    const res = await fetch(this.endpoint, {
      method: 'POST', headers: this.headers(), body: JSON.stringify(message),
    });
    if (!res.ok && res.status !== 202) {
      throw new Error(`MCP ${message.method}: HTTP ${res.status} ${await res.text()}`);
    }
    const sid = res.headers.get('mcp-session-id');
    if (sid) this.session = sid;
    if (message.id === undefined) return null;
    const text = await res.text();
    const type = res.headers.get('content-type') || '';
    const messages = type.includes('text/event-stream')
      ? text.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim())
        .filter(Boolean).map((d) => JSON.parse(d))
      : [JSON.parse(text)];
    const reply = messages.find((m) => m.id === message.id);
    if (!reply) throw new Error(`MCP ${message.method}: no reply in ${text.slice(0, 200)}`);
    if (reply.error) throw new Error(`MCP ${message.method}: ${JSON.stringify(reply.error)}`);
    return reply.result;
  }

  async connect() {
    await this.post({
      jsonrpc: '2.0', id: this.nextId++, method: 'initialize',
      params: {
        protocolVersion: '2025-06-18', capabilities: {},
        clientInfo: { name: 'openpaperdesign-browser-check', version: '1' },
      },
    });
    await this.post({ jsonrpc: '2.0', method: 'notifications/initialized' });
    return this;
  }

  /** Call a tool; returns { ok, text }. */
  async call(name, args = {}) {
    const r = await this.post({
      jsonrpc: '2.0', id: this.nextId++, method: 'tools/call', params: { name, arguments: args },
    });
    const text = (r.content || []).filter((c) => c.type === 'text').map((c) => c.text).join('\n');
    return { ok: r.isError !== true, text };
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [url, tool, args = '{}'] = process.argv.slice(2);
  if (!url || !tool) {
    console.error("usage: mcp-client.mjs '<url with #token=>' <tool> '<json args>'");
    process.exit(2);
  }
  const c = await new McpClient(url).connect();
  const r = await c.call(tool, JSON.parse(args));
  console.log(r.text);
  process.exit(r.ok ? 0 : 1);
}
