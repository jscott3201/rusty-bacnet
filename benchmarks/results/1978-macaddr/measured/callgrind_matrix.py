#!/usr/bin/env python3
import json,subprocess,datetime,hashlib,re,sys
from pathlib import Path
root=Path('/tmp/rusty-batch10-macaddr-measurement'); inputs=json.loads((root/'inputs.json').read_text()); out=root/'outputs/callgrind-final';out.mkdir(exist_ok=False)
manifest={'order':list('ABBA')*3,'runs':[],'inputs':inputs}
def save(): (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def verify():
 for relative,expected in inputs['common'].items(): assert sha(root/relative)==expected,relative
 for v in inputs['variants'].values():
  for relative,expected in v['source_hashes'].items(): assert sha(Path(v['root'])/relative)==expected,relative
try:
 verify()
 for runtime in ['ct','mt']:
  counts=dict(A=0,B=0)
  for pos,label in enumerate(manifest['order'],1):
   counts[label]+=1; v=inputs['variants'][label]
   for n in [1000,10000]:
    name=f'{runtime}-{pos:02}-{label}{counts[label]}-{n}'
    command=['docker','run','--rm','--name','rb-batch10-'+name,'-v',v['root']+':/wt:ro','-v',str(root/'harness')+':/h:ro','-v',v['volume']+':/target:ro','-v',str(out)+':/out',inputs['image'],'sh','-c',f'test "$(sha256sum /target/release/rp-harness | cut -d " " -f 1)" = "{v["linux_binary_sha256"]}" && exec valgrind --tool=callgrind --callgrind-out-file=/out/{name}.cg /target/release/rp-harness {n} {runtime} rp']
    record={'name':name,'runtime':runtime,'position':pos,'label':label,'ordinal':counts[label],'n':n,'command':command,'started':datetime.datetime.now(datetime.timezone.utc).isoformat()};manifest['runs'].append(record);save();print('START',name,flush=True)
    with (out/(name+'.out')).open('w') as stdout,(out/(name+'.err')).open('w') as stderr: result=subprocess.run(command,stdout=stdout,stderr=stderr)
    record.update(exit_code=result.returncode,ended=datetime.datetime.now(datetime.timezone.utc).isoformat());save()
    if result.returncode: raise RuntimeError(f'{name} failed, no retry')
    cg=(out/(name+'.cg')).read_text();err=(out/(name+'.err')).read_text()
    record['instructions']=int(re.search(r'^summary: (\d+)',cg,re.M)[1])
    match=re.search(r'RESULT .* allocs=(\d+) bytes=(\d+) ',err);record['allocs']=int(match[1]);record['bytes']=int(match[2])
    record['raw_hashes']={suffix:sha(out/(name+suffix)) for suffix in ['.cg','.out','.err']};save();print('END',name,record['instructions'],flush=True)
 verify();manifest['completed']=True;save()
except BaseException as error: manifest['failure']=repr(error);save();raise
