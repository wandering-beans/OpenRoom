#!/usr/bin/env python3
"""Exercise the packaged render worker, including aliases and overwrite refusal."""
import hashlib, json, pathlib, shutil, subprocess, sys, tempfile, time
exe=pathlib.Path(sys.argv[1]).resolve();assets=pathlib.Path(sys.argv[2]).resolve()
results=[]
expected={'.cr2','.cr3','.crw','.arw','.sr2','.srf','.raf','.heic'} if '--added-formats' not in sys.argv else {'.tiff','.avif','.psd','.psb'}
if '--camera-formats' in sys.argv:
    expected = {'.nef', '.nrw', '.rw2', '.orf', '.pef', '.rwl', '.srw', '.3fr', '.fff', '.iiq'}
with tempfile.TemporaryDirectory() as directory:
    work=pathlib.Path(directory)
    def run(source,dest,bound=384):
        return subprocess.run([str(exe),'--render-media',str(source),str(dest),str(bound)],capture_output=True,text=True,timeout=65,cwd=work)
    for source in assets.iterdir():
        if source.suffix.lower() not in expected:continue
        before=hashlib.sha256(source.read_bytes()).hexdigest();destination=work/(source.name+'.png')
        started=time.perf_counter();result=run(source,destination)
        assert result.returncode==0,(source.name,result.stderr)
        info=json.loads(result.stdout);info.update(format=source.suffix.lower(),elapsed_ms=(time.perf_counter()-started)*1000)
        assert info['render_width']<=384 and info['render_height']<=384
        saved=destination.read_bytes();assert run(source,destination).returncode!=0
        assert saved==destination.read_bytes()
        assert hashlib.sha256(source.read_bytes()).hexdigest()==before
        # Each decoder must reject an incomplete header in an isolated worker.
        broken_sample=work/('truncated-'+source.name)
        broken_sample.write_bytes(source.read_bytes()[:12])
        failed=run(broken_sample,work/('invalid-'+source.name+'.png'))
        assert failed.returncode!=0,(source.name,'accepted truncated header')
        if '--added-formats' in sys.argv or '--camera-formats' in sys.argv:
            full=run(source,work/('full-'+source.name+'.png'),4096)
            assert full.returncode==0,(source.name,full.stderr)
            full_info=json.loads(full.stdout)
            assert 0<full_info['render_width']<=4096 and 0<full_info['render_height']<=4096
        results.append(info)
        if source.suffix.lower()=='.heic':
            for ext in ['HEIF','HIF']:
                alias=work/('日本語 sample.'+ext);shutil.copyfile(source,alias)
                assert run(alias,work/(ext+'.png')).returncode==0
    broken=work/'broken.CR3';broken.write_bytes(b'not an image')
    assert run(broken,work/'bad.png').returncode!=0
assert {entry['format'] for entry in results} == expected
print(json.dumps(results,indent=2))
