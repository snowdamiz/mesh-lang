"""Scratch-only original-authority embedding; native code pinned for process life."""
import ctypes as c
import hashlib
import json
import pathlib
import struct
import threading
import time
P,U,I=c.c_void_p,c.c_uint64,c.c_int32
class Bytes(c.Structure): _fields_=[('data',P),('len',U)]
Callback=c.CFUNCTYPE(I,P,P,U,P,U,c.POINTER(U))
Lifetime=c.CFUNCTYPE(None,P)
class V1(c.Structure): _fields_=[('version',c.c_uint32),('size',c.c_uint32),('context',P),('calls',Callback*9)]
class V2(c.Structure): _fields_=[('version',c.c_uint32),('size',c.c_uint32),('callbacks',V1),('retain',Lifetime),('release',Lifetime)]
class Runtime:
 def __init__(self,path,scheduled):
  self.lib=c.CDLL(str(path));self.scheduled=scheduled
  def api(name,args,result=I):
   f=getattr(self.lib,name);f.argtypes=args;f.restype=result;return f
  self.api=api
  self.free=api('mesh_library_free_returned_bytes',[c.POINTER(Bytes)],None)
  self.create=api('mesh_authority_domain_create_v2',[c.POINTER(P)])
  self.node=api('mesh_authority_generation_v2',[P,c.POINTER(P)])
  self.bind=api('mesh_authority_bind_v2',[P,c.POINTER(P),U,c.POINTER(V2),c.POINTER(P)])
  self.retire=api('mesh_authority_retire_v2',[P])
  self.drop_node=api('mesh_authority_generation_release_v2',[P],None)
  self.drop_auth=api('mesh_authority_release_v2',[P],None)
  self.scoped=api('mesh_library_call_scoped_v2',[P,P,P,U,c.POINTER(Bytes)])
  if scheduled:
   self.take=api('mesh_library_invocation_take_v2',[P,c.POINTER(Bytes)])
   self.release=api('mesh_library_invocation_release_v2',[P],None)
  assert api('mesh_library_enable_protected_v2',[U])(1)==0
  assert api('mesh_library_init',[])()==0
  self.domain=P();assert self.create(c.byref(self.domain))==0
 def close(self):
  assert self.api('mesh_library_shutdown',[])()==0
  self.api('mesh_authority_domain_release_v2',[P],None)(self.domain)
class OwnedNative:
 def __init__(self,runtime,root,custody):
  self.runtime=runtime;self.root=root;self.custody=custody;custody.mkdir(exist_ok=True)
  self.errors=[];self.lifetimes=[0,0];self.mutations=[]
  def host(operation):
   @Callback
   def callback(_,pointer,length,output,capacity,written):
    written[0]=0
    try:
     data=c.string_at(pointer,length)
     if operation==1:
      size=struct.unpack('>I',data[:4])[0]
      if not 0<size<len(data)-4:return 1
      path=custody/hashlib.sha256(data[4:4+size]).hexdigest();temporary=path.with_suffix('.tmp')
      temporary.write_bytes(data[4+size:]);temporary.chmod(0o600);temporary.replace(path)
      self.mutations.append(('put',path.name));result=b''
     elif operation in (2,3):
      path=custody/hashlib.sha256(data).hexdigest()
      if not path.exists():return 2
      result=path.read_bytes() if operation==2 else b''
      if operation==3:path.unlink();self.mutations.append(('delete',path.name))
     elif operation==4:
      resource={b'messenger/config/v1':'security',b'messenger/network/v1':'network',b'messenger/ohttp/v1':'ohttp',b'messenger/rpc-tls-roots/v1':'rpc-tls'}.get(data)
      if resource is None:return 2
      if not (root/resource).exists():return 2
      result=(root/resource).read_bytes()
     else:return 2
     if len(result)>capacity:return 4
     if result:c.memmove(output,result,len(result))
     written[0]=len(result);return 0
    except BaseException as error:
     self.errors.append(repr(error));return 5
   return callback
  self.callbacks=[host(i) for i in range(1,10)]
  @Lifetime
  def retain(_):self.lifetimes[0]+=1
  @Lifetime
  def release(_):self.lifetimes[1]+=1
  self.retain=retain;self.release=release
  self.v2=V2(2,c.sizeof(V2),V1(1,c.sizeof(V1),None,(Callback*9)(*self.callbacks)),retain,release)
 def owner(self):
  node,auth=P(),P();r=self.runtime
  assert r.node(r.domain,c.byref(node))==0
  assert r.bind(r.domain,(P*1)(node),1,c.byref(self.v2),c.byref(auth))==0
  return node,auth
 def result(self,symbol,data,owner=None):
  r=self.runtime;own=owner or self.owner();output=Bytes()
  try:
   if r.scheduled:
    submit=r.api(symbol+'_submit_v2',[P,P,U,c.POINTER(P)])
    handle=P();status=submit(own[1],data,len(data),c.byref(handle))
    if status:return status,b''
    try:
     until=time.monotonic()+20
     while True:
      status=r.take(handle,c.byref(output))
      if status!=12:break
      assert time.monotonic()<until,'owned completion timeout'
      time.sleep(.0005)
    finally:r.release(handle)
   else:status=r.scoped(own[1],getattr(r.lib,symbol),data,len(data),c.byref(output))
   return status,c.string_at(output.data,output.len) if output.data else b''
  finally:
   r.free(c.byref(output))
   if owner is None:r.drop_auth(own[1]);r.drop_node(own[0])
 def invoke(self,symbol,data,rejected=False):
  result=self.result(symbol,data)
  assert result[0]==(9 if rejected else 0),(symbol,result[0],len(result[1]),self.errors)
  return result[1]
 def close(self):pass
