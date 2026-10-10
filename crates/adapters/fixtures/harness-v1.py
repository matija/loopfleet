import json, os, sys
scenario, agent = sys.argv[1:3]
codex = agent == 'codex'
with open('pid', 'w') as f:
    f.write(str(os.getpid()))
def read():
    return json.loads(sys.stdin.readline())
def emit(value):
    print(json.dumps(value), flush=True)
def reply(request, **fields):
    emit(dict(id=request['id'], **fields) if codex else dict(type='response', id=request['id'], command=request['type'], **fields))
def complete():
    emit(dict(method='turn/completed', params=dict(threadId='thread', turn=dict(id='turn', status='completed'))) if codex else dict(type='agent_settled', aborted=False))
if codex:
    assert sys.argv[3:] == ['app-server', '--listen', 'stdio://']
    request = read()
    assert request['id'] == 1 and request['method'] == 'initialize'
    assert request['params']['clientInfo']['name'] == 'loopfleet'
    reply(request, result={})
    assert read() == dict(method='initialized')
    request = read()
    assert request['id'] == 2 and request['method'] == 'thread/start'
    assert os.path.samefile(request['params']['cwd'], os.getcwd())
    assert request['params']['model'] == 'test-model'
    reply(request, result=dict(thread=dict(id='thread')))
    request = read()
    assert request == dict(id=3, method='turn/start', params=dict(threadId='thread', input=[dict(type='text', text='initial\nprompt')]))
    reply(request, result=dict(turn=dict(id='turn')))
    emit(dict(method='turn/started', params=dict(threadId='thread', turn=dict(id='turn'))))
else:
    assert sys.argv[3:] == ['--mode', 'rpc', '--model', 'test-model']
    request = read()
    assert request == dict(id='0', type='set_steering_mode', mode='one-at-a-time')
    reply(request, success=True)
    request = read()
    assert request == dict(id='1', type='prompt', message='initial\nprompt')
    reply(request, success=True, data=dict(disposition='started'))
    emit(dict(type='turn_start'))
request = read()
if codex:
    assert request == dict(id='steer:tap', method='turn/steer', params=dict(threadId='thread', expectedTurnId='turn', input=[dict(type='text', text='steer')]))
    emit(dict(method='item/completed', params=dict(threadId='thread', turnId='stale', item=dict(type='agentMessage', text='stale'))))
    emit(dict(method='turn/completed', params=dict(threadId='thread', turn=dict(id='stale', status='completed'))))
    emit(dict(method='item/completed', params=dict(threadId='thread', turnId='turn', item=dict(type='userMessage', text='steer'))))
else:
    assert request == dict(id='2', type='steer', message='steer')
    emit(dict(type='message_end', message=dict(role='user', content=[dict(type='text', text='steer')])) )
    emit(dict(type='agent_end', willRetry=False))
    emit(dict(type='auto_compaction_start'))
    emit(dict(type='auto_compaction_end'))
    emit(dict(type='auto_retry_start'))
    emit(dict(type='auto_retry_end', success=True))
if scenario == 'exit':
    print('scripted exit', file=sys.stderr, flush=True)
    sys.exit(7)
if scenario == 'completion-first':
    complete()
if scenario != 'unknown':
    if codex:
        reply(request, **(dict(error=dict(message='rejected')) if scenario == 'rejected' else dict(result=dict(turnId='stale' if scenario == 'stale' else 'turn'))))
    else:
        reply(request, **(dict(success=False, error='rejected') if scenario == 'rejected' else dict(success=True, data=dict(disposition='invalid' if scenario == 'stale' else 'queued'))))
if scenario != 'completion-first':
    emit(dict(method='item/completed', params=dict(threadId='thread', turnId='turn', item=dict(type='agentMessage', text='answer'))) if codex else dict(type='message_end', message=dict(role='assistant', content=[dict(type='text', text='answer')])) )
    complete()
if scenario == 'unknown' and not codex:
    sys.exit(0)
sys.stdin.read()
with open('eof', 'w') as f:
    f.write('closed')
