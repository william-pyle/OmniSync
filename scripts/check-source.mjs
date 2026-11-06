import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const ignored=new Set(['.git','node_modules','target','dist','out','test-results','playwright-report','.local','gen']);
const files=[];
function walk(dir){for(const entry of fs.readdirSync(dir,{withFileTypes:true})){if(ignored.has(entry.name))continue;const full=path.join(dir,entry.name);if(entry.isDirectory())walk(full);else files.push(full);}}
walk(root);const problems=[];const privatePrefixes=[process.env.USERPROFILE,path.resolve(root,'../..')].filter(Boolean).map(s=>s.replaceAll('\\','/').toLowerCase());
for(const full of files){const relative=path.relative(root,full).replaceAll('\\','/');if(/\.(pfx|p12|key|sqlite|db|log)$/i.test(relative)||/^(?:\.env|credentials|tokens)(?:\.|$)/i.test(path.basename(full))&&path.basename(full)!=='.env.example')problems.push(`Private/runtime file: ${relative}`);if(/\.(rs|ts|mjs|json|toml|md|ps1|nsi|html|css|yml|in)$/.test(full)){const source=fs.readFileSync(full,'utf8');if(/-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/.test(source))problems.push(`Private key: ${relative}`);if(privatePrefixes.some(prefix=>source.replaceAll('\\\\','/').replaceAll('\\','/').toLowerCase().includes(prefix)))problems.push(`Nonportable personal path: ${relative}`);if(/"(?:refresh_token|access_token|client_secret)"\s*:\s*"[^"\s]{12,}"/.test(source))problems.push(`Credential-like literal: ${relative}`);}}
for(const required of ['README.md','Cargo.lock','package-lock.json','installer/OmniSync.nsi','assets/icons/app.ico','src/main.ts','docs/verification.json'])if(!fs.existsSync(path.join(root,required)))problems.push(`Missing ${required}`);
for(let i=0;i<24;i++)if(!fs.existsSync(path.join(root,`assets/tray/frame-${String(i).padStart(2,'0')}.png`)))problems.push(`Missing tray frame ${i}`);
const env=fs.readFileSync(path.join(root,'.env.example'),'utf8');for(const line of env.split(/\r?\n/)){if(line.trim()&&!line.startsWith('#')&&!/^[A-Z_]+=$/.test(line))problems.push('.env.example must contain names and empty values only');}
if(problems.length){console.error(problems.join('\n'));process.exitCode=1;}else console.log(`Source audit passed: ${files.length} source/asset files; no private runtime files or embedded credentials detected.`);
