import http.client
import json
import threading
import unittest
from unittest.mock import patch
import adapter

class Tests(unittest.TestCase):
    def setUp(self):
        self.server = adapter.Server(('127.0.0.1', 0), 12345, 'secret')
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
    def tearDown(self):
        self.server.shutdown(); self.server.server_close(); self.thread.join()
    def request(self, method, path, body=None, auth='Bearer secret', extra=None):
        conn = http.client.HTTPConnection('127.0.0.1', self.server.server_port, timeout=3)
        headers = {'Authorization': auth}
        if extra: headers.update(extra)
        conn.request(method, path, body, headers)
        res = conn.getresponse(); body = json.loads(res.read()); status = res.status
        conn.close(); return status, body
    def test_forward_order_alias_and_mapping(self):
        sent = []
        def fake(port, method, path, body=None, timeout=120):
            sent.append(json.loads(body))
            return {'model':'rl-agent','answers':{'q':{'type':'noul','noul':0.2,'rl_agent':{'act_probability':1}}},'usage':{'input_tokens':4}}
        payload = {'model':'laya','state':{'b':1,'a':2},'questions':{'z':{},'a':{}}}
        with patch.object(adapter, 'worker_request', fake):
            status, result = self.request('POST','/v1/systemone', json.dumps(payload))
        self.assertEqual(status,200)
        self.assertNotIn('model',sent[0]); self.assertEqual(list(sent[0]['state']), ['b','a']); self.assertEqual(list(sent[0]['questions']),['z','a'])
        self.assertEqual(result['model'],'laya'); self.assertEqual(result['answers']['q']['confidence'],.8); self.assertEqual(result['answers']['q']['action'],{'act_probability':1})
    def test_auth_validation_and_limits(self):
        self.assertEqual(self.request('POST','/v1/systemone','{}',auth='wrong')[0],401)
        self.assertEqual(self.request('POST','/v1/systemone',json.dumps({'model':'other','state':'','questions':{}}))[0],400)
        self.assertEqual(self.request('POST','/v1/systemone','',extra={'Content-Length':str(adapter.MAX_BODY+1)})[0],413)
        self.assertEqual(self.request('POST','/v1/systemone',json.dumps({'state':'','questions':{str(i):{} for i in range(9)}}))[0],400)
    def test_failure_and_busy(self):
        body=json.dumps({'state':'','questions':{}})
        with patch.object(adapter,'worker_request',side_effect=OSError('failure')):
            self.assertEqual(self.request('POST','/v1/systemone',body)[0],502)
            self.assertEqual(self.request('GET','/health')[0],503)
        self.server.inference.acquire()
        try:
            with patch.object(adapter, 'QUEUE_WAIT', .01):
                self.assertEqual(self.request('POST','/v1/systemone',body)[0],503)
        finally: self.server.inference.release()
    def test_queued_request_waits_then_succeeds(self):
        body=json.dumps({'state':'','questions':{}})
        self.server.inference.acquire()
        timer=threading.Timer(.1,self.server.inference.release); timer.start()
        with patch.object(adapter,'worker_request',return_value={'answers':{},'usage':{}}):
            with patch.object(adapter,'QUEUE_WAIT',1):
                self.assertEqual(self.request('POST','/v1/systemone',body)[0],200)
        timer.join()
    def test_structured_criteria_and_score_legends(self):
        request={'questions':{
            'c':{'type':'choice','criteria':{'z':False,'n':0,'o':{'b':True,'a':[1,None]},'empty':None}},
            's':{'type':'score','criteria':[{'b':False,'a':1},0,None,'fine']},
            'n':{'type':'noul','criteria':{'false':False,'true':{'x':0}}}}}
        legends=adapter.prepare_request(request)
        self.assertEqual(request['questions']['c']['criteria'],{'z':'false','n':'0','o':'{"b": true, "a": [1, null]}','empty':None})
        self.assertEqual(request['questions']['s']['criteria'],['{"b": false, "a": 1}','0','null','fine'])
        self.assertEqual(request['questions']['n']['criteria'],{'false':'false','true':'{"x": 0}'})
        response={'answers':{'s':{'type':'score','legend':{'0':'serialized'}}}}
        self.assertEqual(adapter.normalize(response,legends)['answers']['s']['legend'],{'0':{'b':False,'a':1},'1':0,'2':None,'3':'fine'})
    def test_signal_during_startup_terminates_owned_child(self):
        import signal, socket, sys
        from unittest.mock import Mock
        def unused_port():
            with socket.socket() as sock:
                sock.bind(('127.0.0.1',0)); return sock.getsockname()[1]
        port,worker=unused_port(),unused_port()
        while worker == port: worker=unused_port()
        child=Mock(pid=123, **{'poll.return_value':None})
        handlers={}
        def install(sig,handler): handlers[sig]=handler
        def interrupted(*args,**kwargs):
            handlers[signal.SIGTERM](signal.SIGTERM,None)
            raise OSError('not ready')
        with patch.object(sys,'argv',['adapter.py','--port',str(port),'--worker-port',str(worker)]), \
             patch.object(adapter,'load_runtime',return_value=(adapter.Path('/python'),adapter.Path('/serve'),adapter.MODEL)), \
             patch.object(adapter.subprocess,'Popen',return_value=child) as spawn, \
             patch.object(adapter.signal,'signal',side_effect=install), \
             patch.object(adapter,'worker_request',side_effect=interrupted), \
             patch.object(adapter.time,'sleep'):
            with self.assertRaisesRegex(RuntimeError,'Interrupted'):
                adapter.main()
        child.terminate.assert_called_once(); child.wait.assert_called_once()
        argv = spawn.call_args.args[0]
        self.assertTrue(argv[1].endswith('/worker.py'))
        self.assertEqual(argv[argv.index('--max-questions') + 1], '8')
    def test_budget_error_is_safe_and_other_worker_errors_hidden(self):
        import io, urllib.error
        body=json.dumps({'state':'','questions':{}})
        def err(message):
            return urllib.error.HTTPError('http://127.0.0.1',400,'bad',{},io.BytesIO(json.dumps({'error':message}).encode()))
        with patch.object(adapter,'worker_request',side_effect=err('Question exceeds state context limit (42 tokens)')):
            status, result=self.request('POST','/v1/systemone',body)
            self.assertEqual(status,422); self.assertEqual(result['error'],'Question exceeds state context limit (42 tokens)')
        with patch.object(adapter,'worker_request',side_effect=err('Question exceeds state context limit (42 tokens) secret')):
            status, result=self.request('POST','/v1/systemone',body)
            self.assertEqual(status,400); self.assertNotIn('secret',result['error'])
    def test_worker_bypasses_ambient_proxy(self):
        import os, urllib.request
        with patch.dict(os.environ, {'http_proxy':'http://127.0.0.1:9','no_proxy':''}):
            opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
            proxies=[h for h in opener.handlers if isinstance(h,urllib.request.ProxyHandler)]
            self.assertTrue(all(not h.proxies for h in proxies))
            with patch.object(adapter.WORKER_OPENER,'open',side_effect=OSError('expected')) as opened:
                with self.assertRaises(OSError): adapter.worker_request(12345,'GET','/health')
                self.assertEqual(opened.call_args.args[0].full_url,'http://127.0.0.1:12345/health')
    def test_rebind_after_server_close_and_refuse_active_listener(self):
        import socket
        listener=adapter.Server(('127.0.0.1',0),12345,'')
        port=listener.server_port
        with socket.socket() as probe:
            probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
            with self.assertRaises(OSError): probe.bind(('127.0.0.1',port))
        listener.server_close()
        replacement=adapter.Server(('127.0.0.1',port),12345,'')
        replacement.server_close()
    def test_listener_ownership(self):
        import os
        class Child: pid = os.getpid()
        self.assertTrue(adapter.owns_listener(Child(), self.server.server_port))
        self.assertFalse(adapter.owns_listener(Child(), 1))
    def test_health_rejects_wrong_device_or_model(self):
        good={'status':'ok','dtype':'float32','backend':'mlx-omarchy','device':'Device(gpu, 0)','checkpoint':str(adapter.MODEL)}
        adapter.validate_health(good)
        for key,value in [('dtype','float16'),('device','cpu'),('checkpoint','/tmp/other')]:
            with self.assertRaises(ValueError): adapter.validate_health(dict(good,**{key:value}))

if __name__ == '__main__': unittest.main()
