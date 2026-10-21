'use strict';
const cp=require('node:child_process');
const {own,metrics}=require('./architecture-budget.cjs');
const fs=require('node:fs');
const g=(cwd,...args)=>cp.execFileSync('git',args,{cwd,encoding:'utf8',maxBuffer:64000000});
const show=(cwd,ref,p)=>{try{return g(cwd,'show',ref+':'+p)}catch(e){if(e.status===128)return null;throw e}};
const ls=(cwd,ref)=>g(cwd,'ls-tree','-r','--name-only',ref).split('\n').filter(Boolean);
const source=p=>own(p)||(p.startsWith('modules/')&&p.includes('/src/')&&p.endsWith('.rs')&&!/\/(?:tests|test|vendor|generated)\//.test(p));
const wf=p=>p.startsWith('.github/workflows/')&&/\.ya?ml$/.test(p);
function parse(s){
 if(s==null)return {t:'none',p:[]};
 const rows=s.replace(/\r\n/g,'\n').split('\n');
 const idx=rows.findIndex(x=>/^(?:on|'on'|"on"):\s*(?:#.*)?$/.test(x));
 if(idx<0){
   const inline=rows.find(x=>/^(?:on|'on'|"on"):\s*\S/.test(x));
   if(!inline)return {t:'none',p:[]};
   if(!/\bpull_request\b/.test(inline))return {t:'none',p:[]};
   if(/\bpaths(?:-ignore)?\s*:/.test(inline))throw Error('Inline PR path map requires explicit parsing');
   return {t:'global',p:[]};
 }
 const block=[];for(let j=idx+1;j<rows.length&&!/^\S/.test(rows[j]);j++)block.push(rows[j]);
 const k=block.findIndex(x=>/^  (?:pull_request|'pull_request'|"pull_request"):\s*/.test(x));
 if(k<0)return {t:'none',p:[]};
 const inner=[];for(let j=k+1;j<block.length&&!/^  \S/.test(block[j]);j++)inner.push(block[j]);
 const opts=inner.map((x,i)=>({x,i})).filter(y=>/^    (?:paths|paths-ignore):/.test(y.x));
 if(opts.length>1)throw Error('Multiple PR path filters');
 if(!opts.length)return {t:'global',p:[]};
 const opt=opts[0],type=opt.x.match(/^    (paths(?:-ignore)?):/)[1];
 const tail=opt.x.slice(opt.x.indexOf(':')+1).trim();
 const quote=x=>{let t=x.trim();if((t[0]==="'"&&t.at(-1)==="'")||(t[0]==='"'&&t.at(-1)==='"'))t=t.slice(1,-1);return t};
 let patterns=[];
 if(tail.startsWith('[')&&tail.endsWith(']'))patterns=tail.slice(1,-1).split(',').map(quote).filter(Boolean);
 else if(!tail||tail.startsWith('#')){
   for(let j=opt.i+1;j<inner.length&&!/^    \S/.test(inner[j]);j++){
     if(!inner[j].trim()||/^\s*#/.test(inner[j]))continue;
     const match=inner[j].match(/^\s{6,}-\s+(.+?)\s*$/);
     if(!match)throw Error('Malformed PR path list item');
     patterns.push(quote(match[1]));
   }
 }else throw Error('Unsupported PR paths syntax: '+tail);
 if(!patterns.length||patterns.some(x=>!x||x.startsWith('#')))throw Error('Missing PR paths');
 return {t:type,p:patterns};
}
const regexCache=new Map();
function pathRegex(pattern){
 if(regexCache.has(pattern))return regexCache.get(pattern);
 let out='^';
 for(let i=0;i<pattern.length;i++){
   const c=pattern[i];
   if(c==='*'){
     if(pattern[i+1]==='*'){
       if(pattern[i+2]==='/'){out+='(?:.*/)?';i+=2}
       else{out+='.*';i++}
     }else out+='[^/]*';
   }else if(c==='?')out+='[^/]';
   else if(c==='['){
     const end=pattern.indexOf(']',i+1);if(end<0)throw Error('Unclosed GitHub glob class '+pattern);
     let cls=pattern.slice(i+1,end);
     if(cls.startsWith('!'))cls='^'+cls.slice(1);
     if(!cls||cls==='^')throw Error('Empty glob class '+pattern);
     out+='['+cls+']';i=end;
   }else if(c==='+'){
     if(i===0)throw Error('Invalid GitHub + glob '+pattern);
     out+='+';
   }else if(c==='\\'){
     if(i===pattern.length-1)throw Error('Invalid trailing glob escape '+pattern);
     out+='\\'+pattern[++i];
   }else{
     if('.^$(){}|'.includes(c))out+='\\';
     out+=c;
   }
 }
 const re=new RegExp(out+'$');regexCache.set(pattern,re);return re;
}
function matches(pol,file){
 if(pol.t==='none'||pol.t==='global')return false;
 if(pol.t==='paths-ignore')return !pol.p.some(p=>pathRegex(p).test(file));
 let matched=false;
 for(const item of pol.p){
   const excluded=item.startsWith('!');
   const raw=excluded?item.slice(1):item;
   if(pathRegex(raw).test(file))matched=!excluded;
 }
 return matched;
}
function audit(cwd,base,head){
 const bs=ls(cwd,base),hs=ls(cwd,head),old=new Set(bs),now=new Set(hs),errors=[];
 const diff=g(cwd,'diff','--name-status','-z','--find-renames',base,head).split('\0').filter(Boolean);
 const ch=[];
 for(let i=0;i<diff.length;){const op=diff[i++],from=diff[i++];const to=/^[RC]/.test(op)?diff[i++]:op==='D'?null:from;ch.push({op,from:op==='A'?null:from,to})}
 const oldwf=bs.filter(wf),newwf=hs.filter(wf);
 const cache=new Map();
 const policy=(ref,p)=>{const key=ref+'::'+p;if(!cache.has(key))cache.set(key,parse(show(cwd,ref,p)));return cache.get(key)};
 const count=(ref,files,p)=>files.filter(f=>matches(policy(ref,f),p)).length;
 for(const d of ch)if(d.to&&source(d.to)){
   const after=show(cwd,head,d.to);const before=d.from&&source(d.from)?show(cwd,base,d.from):null;
   if(after==null)throw Error('missing source');
   const n=metrics(after).lines,b=before==null?null:metrics(before).lines;
   if(b!==null&&n>b)errors.push('SOURCE SIZE '+d.to+' '+b+' -> '+n);
   if(b===null&&n>800)errors.push('NEW SOURCE '+d.to+' '+n+' >800');
   if(b===null&&count(head,newwf,d.to)>6)errors.push('NEW FANOUT '+d.to+' >6');
   if(d.from&&d.from!==d.to&&source(d.from)&&count(head,newwf,d.to)>count(base,oldwf,d.from))errors.push('RENAMED FANOUT '+d.to);
 }
 const workflowChanges=ch.filter(d=>[d.from,d.to].some(x=>x&&wf(x)));
 for(const d of workflowChanges)if(d.to&&policy(head,d.to).t==='global'&&(!d.from||policy(base,d.from).t!=='global'))errors.push('NEW GLOBAL PR WORKFLOW '+d.to);
 if(workflowChanges.length)for(const f of bs)if(now.has(f)&&source(f)){
   const b=workflowChanges.filter(d=>d.from&&matches(policy(base,d.from),f)).length;
   const n=workflowChanges.filter(d=>d.to&&matches(policy(head,d.to),f)).length;
   if(n>b)errors.push('FANOUT GROWTH '+f+' '+b+' -> '+n);
   else if(n===b&&n>0){
     const prior=workflowChanges.filter(d=>d.from&&matches(policy(base,d.from),f)).map(d=>d.from);
     const extra=workflowChanges.filter(d=>d.to&&matches(policy(head,d.to),f)&&!prior.includes(d.to));
     if(extra.length)errors.push('NEW WORKFLOW CONSUMER '+f+': '+extra.join(', '));
   }
   if(errors.length>60)break;
 }
 return {errors,workflowChanges:workflowChanges.length,changed:ch.length};
}
if(require.main===module){
 try{
   const args=Object.fromEntries(process.argv.slice(2).map(x=>x.replace(/^--/,'').split('=')));
   const r=audit(process.cwd(),args.base,args.head);
   const summary='CI no-growth: '+r.changed+' changed files, '+r.workflowChanges+' workflow edits; '+r.errors.length+' violation(s)\n'+r.errors.join('\n');
   process.stdout.write(summary+'\n');
   if(process.env.GITHUB_STEP_SUMMARY)fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY,summary+'\n');
   if(r.errors.length)process.exitCode=1;
 }catch(e){console.error(e.message);process.exitCode=1}
}
module.exports={parse,matches,audit,source};
