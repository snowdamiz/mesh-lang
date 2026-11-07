import hashlib,json,pathlib,shutil,subprocess
root=pathlib.Path.cwd(); app=pathlib.Path('/Users/andreyyurlov/.t3/worktrees/whatsdown/t3-f9098070'); product=app/'mesh-private-messenger'; work=root/'.tmp/native-owned-invocations/core-bootstrap'; work.mkdir()
record={'app_head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=app,text=True).strip(),'sources':{}}
for name in ['mobile-core','messenger-protocol']:
 source=product/'packages'/name
 for p in sorted(source.rglob('*')):
  rel=p.relative_to(source)
  if not p.is_file() or p.suffix not in ['.mpl','.toml'] or 'tests' in rel.parts or p.name.endswith('.test.mpl'): continue
  target=work/name/rel; target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,target)
  record['sources'][str(p)]=hashlib.sha256(p.read_bytes()).hexdigest()
 manifest=work/name/'mesh.toml';manifest.write_text(manifest.read_text().replace('../../../mesh-lang/packages/',str(root/'packages')+'/'))
fixture=product/'tests/bootstrap-native-20261106'
shutil.copytree(fixture,work/'driver',ignore=shutil.ignore_patterns('__pycache__'))
for p in fixture.rglob('*'):
 if p.is_file() and p.suffix in ['.mpl','.toml','.c','.py']:record['sources'][str(p)]=hashlib.sha256(p.read_bytes()).hexdigest()
manifest=work/'driver/mesh.toml';manifest.write_text(manifest.read_text().replace('../../packages/mobile-core','../mobile-core'))
(work/'staging.json').write_text(json.dumps(record,indent=2)+'\n')
