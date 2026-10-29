import json,itertools,statistics as st
from pathlib import Path
r=Path('/tmp/rusty-batch10-macaddr-measurement');out={}
def median(values):return st.median(values)
def interval(values):
 values=sorted(values)
 def percentile(q):
  i=(len(values)-1)*q;lo=int(i);hi=min(lo+1,len(values)-1);return values[lo]+(values[hi]-values[lo])*(i-lo)
 return [percentile(.025),percentile(.975)]
def describe(values):return {'values':values,'median':median(values),'range':[min(values),max(values)]}
cg=json.loads((r/'outputs/callgrind-final/manifest.json').read_text());assert cg.get('completed')
for rt in ['ct','mt']:
 metric={};slopes={};rates={}
 for label in ['A','B']:
  rows=[x for x in cg['runs'] if x['runtime']==rt and x['label']==label]
  def by(ordinal,n):return next(x for x in rows if x['ordinal']==ordinal and x['n']==n)
  slopes[label]=[(by(i,10000)['instructions']-by(i,1000)['instructions'])/9000 for i in range(1,7)]
  rates[label]={name:[by(i,10000)[name]/10000 for i in range(1,7)] for name in ['allocs','bytes']}
  metric[label]={'instructions_per_request':describe(slopes[label]),**{name+'_per_request':describe(v) for name,v in rates[label].items()},'allocation_slopes':{name:describe([(by(i,10000)[name]-by(i,1000)[name])/9000 for i in range(1,7)]) for name in ['allocs','bytes']}}
 gains=[a-b for a,b in zip(slopes['A'],slopes['B'])];drift=max(slopes['A'])-min(slopes['A']);alllower=all(x>0 for x in gains);gain=median(slopes['A'])-median(slopes['B'])
 metric.update(paired_instruction_savings=describe(gains),median_instruction_savings=gain,repeated_A_drift=drift,all_six_B_lower=alllower,gain_exceeds_A_drift=gain>drift)
 metric['allocations_nonincreasing']=all(median(rates['B'][name])<=median(rates['A'][name]) for name in ['allocs','bytes'])
 metric['accepted']=alllower and gain>drift and metric['allocations_nonincreasing'];out[rt]=metric
native=json.loads((r/'outputs/native-final/manifest.json').read_text());assert native.get('completed');out['native']={}
for w in native['workloads']:
 values={label:[json.loads((r/'outputs/native-final/criterion'/x['name']/w/'estimates.json').read_text())['median']['point_estimate'] for x in native['runs'] if x['label']==label] for label in ['A','B']}
 bootstrap=[]
 for indices in itertools.product(range(6),repeat=6):
  a=median([values['A'][i] for i in indices]);b=median([values['B'][i] for i in indices]);bootstrap.append(b/a-1)
 ci=interval(bootstrap);deltas=[b-a for a,b in zip(values['A'],values['B'])];pct=[b/a-1 for a,b in zip(values['A'],values['B'])]
 out['native'][w]={'unit':'ns','A':describe(values['A']),'B':describe(values['B']),'paired_difference':describe(deltas),'paired_slowdown_fraction':describe(pct),'aggregate_slowdown_fraction':median(values['B'])/median(values['A'])-1,'paired_bootstrap95_slowdown_fraction':ci,'observed_median_nonincreasing':median(values['B'])<=median(values['A']),'upper_bound_at_most_2percent':ci[1]<=.02,'accepted':median(values['B'])<=median(values['A']) and ci[1]<=.02}
out['accepted']=out['ct']['accepted'] and out['mt']['accepted'] and all(x['accepted'] for x in out['native'].values())
(r/'analysis.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out,indent=2))
