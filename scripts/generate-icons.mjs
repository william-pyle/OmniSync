import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {Resvg} from '@resvg/resvg-js';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const svg=fs.readFileSync(path.join(root,'assets/omnisync.svg'),'utf8');
const icons=JSON.parse(fs.readFileSync(path.join(root,'assets/ui-icons.json'),'utf8'));
const render=(text,size)=>new Resvg(text,{fitTo:{mode:'width',value:size}}).render().asPng();
function ico(text){const sizes=[16,20,24,32,40,48,64,128,256],images=sizes.map(n=>render(typeof text==='function'?text(n):text,n));const header=Buffer.alloc(6+16*sizes.length);header.writeUInt16LE(1,2);header.writeUInt16LE(sizes.length,4);let offset=header.length;images.forEach((png,i)=>{const at=6+i*16;header[at]=sizes[i]===256?0:sizes[i];header[at+1]=header[at];header.writeUInt16LE(1,at+4);header.writeUInt16LE(32,at+6);header.writeUInt32LE(png.length,at+8);header.writeUInt32LE(offset,at+12);offset+=png.length;});return Buffer.concat([header,...images]);}
fs.mkdirSync(path.join(root,'assets/icons'),{recursive:true});fs.mkdirSync(path.join(root,'assets/tray'),{recursive:true});
fs.writeFileSync(path.join(root,'assets/icons/app.ico'),ico(svg));fs.writeFileSync(path.join(root,'assets/icons/app.png'),render(svg,256));
const statuses={backed:['check','#45D6A3'],syncing:['sync','#F28B42'],pending:['clock','#F1C66B'],excluded:['minus','#A7B2C5'],issue:['issue','#FF889A'],paused:['pause','#F1C66B'],offline:['offline','#A7B2C5']};
for(const [name,[glyph,color]] of Object.entries(statuses)){
 // Explorer composites the entire icon canvas; position the small badge ourselves.
 const overlay=size=>{const extent=Math.round(size*(size<=24?.5:.375));return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}"><g transform="translate(0 ${size-extent}) scale(${extent/32})"><circle cx="16" cy="16" r="15" fill="${color}" stroke="#131C30" stroke-width="1.5"/><g transform="translate(4 4)" fill="none" stroke="#131C30" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><path d="${icons.paths[glyph]}"/></g></g></svg>`;};
 fs.writeFileSync(path.join(root,`assets/icons/${name}.ico`),ico(overlay));
 const tray=svg.replace('</svg>',`<circle cx="49" cy="49" r="14" fill="#131C30"/><g transform="translate(38 38) scale(.9)" fill="none" stroke="${color}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="${icons.paths[glyph]}"/></g></svg>`);
 fs.writeFileSync(path.join(root,`assets/tray/${name}.png`),render(tray,32));
}
fs.writeFileSync(path.join(root,'assets/tray/idle.png'),render(svg,32));
for(let i=0;i<24;i++){
 const frame=svg.replace('<g clip-path=',`<g transform="rotate(${i*15} 32 32)" clip-path=`).replace('opacity="0" filter=',`opacity=".58" stroke-dashoffset="${-i*100/12}" filter=`);
 fs.writeFileSync(path.join(root,`assets/tray/frame-${String(i).padStart(2,'0')}.png`),render(frame,32));
}
console.log('Generated Windows ICO resources and 24 native tray frames from the approved SVG.');
fs.mkdirSync(path.join(root,'assets/identity'),{recursive:true});
for(const [name,size] of [['StoreLogo',50],['Logo150',150],['Logo44',44]]) fs.writeFileSync(path.join(root,`assets/identity/${name}.png`),render(svg,size));
