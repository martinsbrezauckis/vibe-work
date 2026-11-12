// Exact nested/document scrolling with final offset readback; assembled by the host with retained node identity.
/* global ENV, REQ, actionRef, observedTarget, describe, visible */
(() => {
  // eslint-disable-next-line no-undef -- Expanded by the script assembler.
  __VW_COMMON__
  const out={url:String(location.href).slice(0,2000)};
  const reply=(status,extra={})=>JSON.stringify({...out,status,...extra});
  if (!ENV) return reply('stale');
  let node=document.scrollingElement;
  if (REQ.ref) {
    const found=actionRef(REQ.ref,true);
    if (!found.target || !observedTarget(found.target,REQ.observedNode,REQ.observedName)) return reply('stale');
    node=found.target;
    if (!visible(node)) return reply('notActionable',{failed:['visible']});
  }
  if (!node) return reply('error',{message:'The scroll container is unavailable. Read the page again.'});
  const position=()=>({x:node.scrollLeft,y:node.scrollTop});
  ENV.scrollStarts ||= new WeakMap();
  let before=ENV.scrollStarts.get(node) || position();
  if (!REQ.postconditionOnly) {
    before=position();ENV.scrollStarts.set(node,before);
    if (REQ.action!=='scroll_position') node.scrollBy({left:REQ.x,top:REQ.y,behavior:'instant'});
  }
  const current=position(),width=node.scrollWidth,height=node.scrollHeight;
  const rtl=node.ownerDocument.defaultView.getComputedStyle(node).direction==='rtl';
  const extentX=Math.max(0,width-node.clientWidth),maxY=Math.max(0,height-node.clientHeight);
  const minX=rtl?-extentX:0,maxX=rtl?0:extentX;
  const edges={left:current.x<=minX+1,right:current.x>=maxX-1,top:current.y<=1,bottom:current.y>=maxY-1};
  const endReached=(REQ.x!==0||REQ.y!==0)&&(!REQ.x||(REQ.x<0?edges.left:edges.right))&&(!REQ.y||(REQ.y<0?edges.top:edges.bottom));
  return reply('done',{target:REQ.ref?describe(node):undefined,scroll:{...current,width,height,viewportWidth:node.clientWidth,viewportHeight:node.clientHeight,minX,maxX,maxY,edges,moved:current.x!==before.x||current.y!==before.y,endReached}});
})()
