#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { compile } from 'json-schema-to-typescript';

// The generator emits only what the root union reaches, and the root is the
// frames that cross the socket. A snapshot section the contract declares is a
// $def no frame references, so each one the web reads is named here and
// emitted beside the root, with the $defs it refers to.
const SNAPSHOT_DEFS = ['usageSnapshot', 'usagePreferences', 'usagePinSetPayload', 'usageHiddenSetPayload', 'uiStateUsageHints', 'focusCheckoutPayload', 'interfaceLanguageSetPayload', 'agentLogoMode', 'agentLogoModeSetPayload', 'paneTurnRowMode', 'paneTurnRowModeSetPayload', 'colorThemeSetPayload', 'terminalFrame', 'agentNotice', 'desktopNoticesPayload'];

const root = path.resolve(process.cwd(), '..');
const schema = JSON.parse(fs.readFileSync(path.join(root, 'contracts/hided-ws.schema.json'), 'utf8'));
// One compiler pass owns shared definitions: usage is reached by both a
// server frame and the standalone section exports, so separate passes would
// declare its types twice. The wrapper is a type-generation root, not wire.
const ts = await compile({
  type: 'object',
  properties: {
    frame: { title: 'HidedWs', oneOf: schema.oneOf },
    sections: { title: 'SnapshotSections', anyOf: SNAPSHOT_DEFS.map((name) => ({ $ref: `#/$defs/${name}` })) },
  },
  $defs: schema.$defs,
}, 'HideWireTypes', { bannerComment: '/* Generated from contracts/hided-ws.schema.json. */', ignoreMinAndMaxItems: true });
const out = path.join(process.cwd(), 'src/generated');
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, 'hided-ws.ts'), ts);
console.log('wrote src/generated/hided-ws.ts');
