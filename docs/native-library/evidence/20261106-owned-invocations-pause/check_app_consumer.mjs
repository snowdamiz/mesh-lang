import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {parseBindings,generate} from '/Users/andreyyurlov/.t3/worktrees/whatsdown/t3-f9098070/mesh-private-messenger/apps/mobile/scripts/generate-scoped-bindings.mjs';
const root=new URL('./',import.meta.url);
const outputs=[];
for(const family of ['app-baseline','app-candidate']) {
  const source=readFileSync(new URL(family+'/libmessenger_mobile.ts',root),'utf8');
  const abi=JSON.parse(readFileSync(new URL(family+'/libmessenger_mobile.abi.json',root),'utf8'));
  const rows=parseBindings(source,abi);
  const scoped=generate(source,abi);
  if(rows.length!==175 || scoped.includes('_submit_v2')) throw Error('legacy_consumer_changed');
  outputs.push(scoped);
  writeFileSync(new URL(family+'/scoped-consumer.ts',root),scoped);
}
if(outputs[0]!==outputs[1]) throw Error('scoped_output_changed');
const result={actual_app_parser:true,public_js_operations:175,scoped_operations:164,byte_identical:true,sha256:createHash('sha256').update(outputs[1]).digest('hex')};
writeFileSync(new URL('app-consumer.json',root),JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify(result));
