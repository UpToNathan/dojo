# Development provenance generator; never used by the desktop runtime.
import json, hashlib, subprocess, difflib, pathlib
root = pathlib.Path(__file__).resolve().parents[2]
planpath = root / 'vendor/cua/copy-plan.json'
plan = json.loads(planpath.read_text())
patch = plan['patches'][0]
paths = {entry['path']: entry.get('added', False) for entry in patch['files']}
up = pathlib.Path.home() / '.cache/dojo/upstream/cua' / plan['revision']
parts, records = [], []
for path, added in paths.items():
    data = (root / 'vendor/cua/upstream' / path).read_bytes()
    before = b'' if added else subprocess.check_output(['git', '-C', str(up), 'show', plan['revision'] + ':' + path])
    parts.append('diff --git a/' + path + ' b/' + path + '\n')
    if added:
        parts.append('new file mode 100644\n')
    parts.extend(difflib.unified_diff(before.decode().splitlines(True), data.decode().splitlines(True), fromfile='/dev/null' if added else 'a/' + path, tofile='b/' + path))
    record = {'path': path, 'sha256': hashlib.sha256(data).hexdigest()}
    if added:
        record['added'] = True
    records.append(record)
file = root / patch['metadata']
file.write_text(''.join(parts))
patch['sha256'] = hashlib.sha256(file.read_bytes()).hexdigest()
patch['files'] = records
planpath.write_text(json.dumps(plan, indent=2) + '\n')
