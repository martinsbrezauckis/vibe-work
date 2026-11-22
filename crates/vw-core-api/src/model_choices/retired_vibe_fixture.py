#!/usr/bin/env python3
import json
import os
import pathlib
import sys
import tomllib
home = pathlib.Path(os.environ['VIBE_HOME'])
def listed(project=False):
    path = home / ('project-models.json' if project and (home / 'project-models.json').exists() else 'models.json')
    rows = json.loads(path.read_text())
    config = home / 'config.toml'
    if config.exists():
        for row in tomllib.loads(config.read_text()).get('models', []):
            rows = [item for item in rows if item['alias'] != row['alias']] + [row]
    return rows
def state():
    return json.loads((home / 'state.json').read_text())
def public(row):
    # Real Vibe 3.1.4 ModelConfigView omits provider, even for activeModel.
    return {key: value for key, value in row.items() if key != 'provider'}
def runtime():
    current = state()
    rows = listed(current.get('project', False))
    # A cold 3.1.4 resume can expose only configured rows until the catalog is used.
    if (home / 'cold-models.json').exists() and not (home / 'catalog-ready').exists() and not current['writes']:
        rows = json.loads((home / 'cold-models.json').read_text())
    row = public(next(row for row in rows if row['alias'] == current['active']))
    row['thinking'] = current['thinking']
    return {'config': {'models': [public(item) for item in rows], 'activeModel': row,
                       'validationWarnings': current['warnings']}}
for line in sys.stdin:
    frame = json.loads(line)
    message = frame['message']
    if message is None:
        print(json.dumps(frame), flush=True)
        continue
    if 'id' not in message:
        continue
    method = message['method']
    params = message.get('params') or {}
    if method == 'initialize':
        result = {'serverInfo': {'name': 'retired-choice-test', 'version': '0'}}
    elif method == 'session/list':
        result = {'items': [], 'nextCursor': None}
    elif method == 'config/read':
        rows = listed(bool(params.get('cwd')))
        default = public(next(row for row in rows if row['alias'] == state()['default']))
        result = {'config': {'models': [public(item) for item in rows],
                             'activeModel': default, 'validationWarnings': []}}
    elif method == 'config/fields/read':
        if not (home / 'wait-to-ready').exists():
            (home / 'catalog-ready').write_text('ready')
        rows = listed(state().get('project', False))
        models = {row['alias']: row for row in rows}
        providers = [{'name': name, 'api_base': 'http://127.0.0.1:19462/v1',
                      'api_style': 'openai', 'backend': 'generic'}
                     for name in {row['provider'] for row in rows}]
        fields = [
            {'name': 'models', 'kind': 'complex', 'description': '', 'value': models,
             'path': '/models'},
            {'name': 'providers', 'kind': 'complex', 'description': '', 'value': providers,
             'path': '/providers'}]
        result = {'fields': [] if (home / 'no-fields').exists() else fields, 'targets': []}
    elif method == 'runtime/read':
        result = {'runtime': runtime(),
                  'ready': not (home / 'wait-to-ready').exists() or (home / 'catalog-ready').exists()}
    elif method == 'session/ready/wait':
        if (home / 'wait-fails').exists():
            result = {'ready': False}
        else:
            (home / 'catalog-ready').write_text('ready')
            result = {'ready': True}
    elif method == 'config/model/write':
        current = state()
        if (home / 'write-fails').exists():
            result = {'runtime': runtime(), 'failures': [{'message': 'temporary model write failure'}]}
        elif any(row['alias'] == params['modelAlias'] for row in listed(current.get('project', False))):
            current['active'] = params['modelAlias']
            current['thinking'] = params['reasoningEffort']
            current['warnings'] = []
        if not (home / 'write-fails').exists():
            current['writes'].append([params['modelAlias'], params['reasoningEffort']])
            (home / 'state.json').write_text(json.dumps(current))
            result = {'runtime': runtime(), 'failures': []}
    else:
        raise AssertionError(method)
    frame['message'] = {'jsonrpc': '2.0', 'id': message['id'], 'result': result}
    print(json.dumps(frame), flush=True)
