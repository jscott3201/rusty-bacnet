#!/usr/bin/env python3
import json,os,subprocess,datetime,hashlib,shutil,sys
from pathlib import Path
from capture import load_inputs
root=Path(sys.argv[1]).resolve(); inputs=load_inputs(root);out=root/'outputs/native-final';out.mkdir(exist_ok=False)
workloads=['bip_read_property_latency','bip_write_property_latency','bip_rpm_10_objects_latency'];manifest={'order':list('ABBA')*3,'runs':[],'inputs':inputs,'workloads':workloads}
def save(): (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def verify():
 for v in inputs['variants'].values():
  assert sha(v['native_binary'])==v['native_binary_sha256']
  for rel,expected in v['source_hashes'].items(): assert sha(Path(v['root'])/rel)==expected,rel
try:
 verify();counts=dict(A=0,B=0)
 for pos,label in enumerate(manifest['order'],1):
  counts[label]+=1; v=inputs['variants'][label];name=f'{pos:02}-{label}{counts[label]}';baseline='1978-macaddr-'+name
  env=os.environ.copy();env.update(TOKIO_WORKER_THREADS='2',CARGO_TARGET_DIR=v['native_target'])
  assert shutil.disk_usage(root).free>=50*1024**3
  for w in workloads: assert not (Path(v['native_target'])/'criterion'/w/baseline).exists()
  command=[v['native_binary'],'--bench','^('+'|'.join(workloads)+')$','--sample-size','100','--warm-up-time','3','--measurement-time','5','--noplot','--save-baseline',baseline]
  record={'name':name,'position':pos,'label':label,'ordinal':counts[label],'command':command,'cwd':v['root'],'native_target':v['native_target'],'TOKIO_WORKER_THREADS':2,'started':datetime.datetime.now(datetime.timezone.utc).isoformat(),'load_before':os.getloadavg(),'free_bytes_before':shutil.disk_usage(root).free,'power_before':subprocess.check_output(['pmset','-g','batt'],text=True)};manifest['runs'].append(record);save();print('START',name,flush=True)
  with (out/(name+'.out')).open('w') as stdout,(out/(name+'.err')).open('w') as stderr: result=subprocess.run(command,cwd=v['root'],env=env,stdout=stdout,stderr=stderr)
  record.update(exit_code=result.returncode,ended=datetime.datetime.now(datetime.timezone.utc).isoformat(),load_after=os.getloadavg());save()
  if result.returncode:raise RuntimeError(f'{name} failed, no retry')
  for w in workloads:
   source=Path(v['native_target'])/'criterion'/w/baseline;dest=out/'criterion'/name/w;shutil.copytree(source,dest)
   sample=json.loads((dest/'sample.json').read_text());assert len(sample['iters'])==len(sample['times'])==100
   assert all(x>0 for x in sample['iters']+sample['times'])
  record['raw_hashes']={str(p.relative_to(out)):sha(p) for p in (out/'criterion'/name).rglob('*.json')};save();print('END',name,flush=True)
 verify();manifest['completed']=True;save()
except BaseException as error:manifest['failure']=repr(error);save();raise
