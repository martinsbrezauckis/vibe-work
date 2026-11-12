// Read-only wait predicates; assembled with common.js and the retained document/ref binding by the host.
/* global ENV, REQ, collect, nodeIdentity, visible */
(() => {
  // eslint-disable-next-line no-undef -- Expanded by the script assembler.
  __VW_COMMON__
  const out={url:String(location.href).slice(0,8192),met:false};
  if (!ENV) return JSON.stringify(out);
  if (REQ.url) out.met=location.href===REQ.url;
  else if (REQ.load) out.met=document.readyState==='complete'||REQ.load==='interactive'&&document.readyState==='interactive';
  else {
    let nodes;
    if (REQ.ref) {
      const all=collect();
      nodes=all.elements.filter(el=>nodeIdentity(el)===REQ.observedNode);
      if (!nodes.length && all.truncated) return JSON.stringify({...out,invalid:true});
    } else {
      try { nodes=Array.from(document.querySelectorAll(REQ.selector)); }
      catch { return JSON.stringify({...out,invalid:true}); }
    }
    const attached=nodes.some(n=>n.isConnected),shown=nodes.some(n=>n.isConnected&&visible(n));
    out.met=REQ.state==='attached'?attached:REQ.state==='detached'?!attached:REQ.state==='hidden'?!shown:shown;
  }
  return JSON.stringify(out);
})()
