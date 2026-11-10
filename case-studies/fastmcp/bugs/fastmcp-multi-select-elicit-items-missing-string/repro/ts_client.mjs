// npm install @modelcontextprotocol/sdk; then: node ts_client.mjs python server.py
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { ElicitRequestSchema } from "@modelcontextprotocol/sdk/types.js";

const client = new Client({ name: "ts", version: "1" }, { capabilities: { elicitation: {} } });
client.setRequestHandler(ElicitRequestSchema, async () => ({ action: "accept", content: { value: ["red"] } }));
await client.connect(new StdioClientTransport({ command: process.argv[2], args: [process.argv[3]] }));
console.log(JSON.stringify(await client.callTool({ name: "pick", arguments: {} })));
await client.close();
