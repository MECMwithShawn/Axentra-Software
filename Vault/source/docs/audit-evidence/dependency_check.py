import json,tomllib,pathlib,subprocess,collections
root=pathlib.Path(__file__).resolve().parents[2]
work=root/'target'/'audit'
lock=tomllib.loads((root/'Cargo.lock').read_text())
meta=json.loads((work/'metadata-windows.json').read_text(encoding='utf-8-sig'))
nodes={n['id']:n for n in meta['resolve']['nodes']}
seen=set()
def visit(pid):
    if pid in seen:return
    seen.add(pid)
    for d in nodes.get(pid,{}).get('deps',[]):
        if any(k['kind']!='dev' for k in d['dep_kinds']):visit(d['pkg'])
for pid in meta['workspace_members']:visit(pid)
windows={(p['name'],p['version']) for p in meta['packages'] if p['id'] in seen}
advisories=[]
for p in next((work/'rustsec').glob('advisory-db-*')).glob('crates/*/*.md'):
    s=p.read_text(encoding='utf-8')
    if '```toml' not in s:continue
    obj=tomllib.loads(s.split('```toml',1)[1].split('```',1)[0])
    a=obj['advisory']
    if a.get('withdrawn'):continue
    for package in lock['package']:
        if package['name']==a['package']:
            advisories.append({'id':a['id'],'package':a['package'],'version':package['version'],'date':str(a['date']),'title':s.split('\n# ',1)[-1].splitlines()[0],'informational':a.get('informational'),'url':a.get('url'),'windows':(package['name'],package['version']) in windows,'versions':obj.get('versions',{}),'affected':obj.get('affected',{})})
inputfile=work/'advisory-candidates.json'
inputfile.write_text(json.dumps(advisories,default=str))
exe=root/'target'/'debug'/'audit-semver.exe'
result=subprocess.run([str(exe),str(inputfile)],check=True,capture_output=True,text=True)
matches=json.loads(result.stdout)
packages=[p for p in meta['packages'] if p['id'] in seen]
license_counts=collections.Counter(p['license'] or 'UNSPECIFIED' for p in packages)
duplicates=collections.defaultdict(list)
for p in packages:duplicates[p['name']].append(p['version'])
indexfiles=list((pathlib.Path.home()/'.cargo'/'registry'/'index').glob('*/.cache'))
yanked=[]; unchecked=[]
for p in lock['package']:
    if not p.get('source','').startswith('registry+'):continue
    name=p['name']; rel=(('1/'+name) if len(name)==1 else ('2/'+name) if len(name)==2 else ('3/'+name[0]+'/'+name) if len(name)==3 else name[:2]+'/'+name[2:4]+'/'+name)
    found=False
    for index in indexfiles:
        f=index/rel
        if not f.exists():continue
        for part in f.read_bytes().split(b'\0'):
            if part.startswith(b'{'):
                entry=json.loads(part)
                if entry.get('vers')==p['version']:
                    found=True
                    if entry.get('yanked'):yanked.append({'package':name,'version':p['version']})
                    break
    if not found:unchecked.append({'package':name,'version':p['version']})
report={'rustsec_revision':json.loads((work/'rustsec-head.json').read_text(encoding='utf-8-sig'))['sha'],'lock_packages':len(lock['package']),'windows_normal_and_build_packages':len(packages),'matching_advisories':matches,'licenses':dict(sorted(license_counts.items())),'missing_license':[p['name'] for p in packages if not p['license']],'non_crates_io_sources':[{'name':p['name'],'source':p.get('source')} for p in lock['package'] if p.get('source') and p['source']!='registry+https://github.com/rust-lang/crates.io-index'],'duplicates':{k:v for k,v in duplicates.items() if len(v)>1},'cached_index_yanked':yanked,'cached_index_unchecked':unchecked,'limitations':['Yanked status is from the local cached index, not refreshed.','No project deny.toml exists; license expressions and duplicate versions are inventoried, not approved by a policy.','Windows set excludes dev edges and includes build dependencies; a match is not proof of an exploitable reachable code path.']}
out=root/'docs'/'audit-evidence'/'dependency-results.json';out.write_text(json.dumps(report,indent=2))
print(json.dumps({k:v for k,v in report.items() if k not in ('licenses','duplicates')},indent=2))


