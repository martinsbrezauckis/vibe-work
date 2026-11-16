// The same main-world completion record as page_script.js, for a page whose Content-Security-Policy refuses eval:
// the expression is part of this script's own source (the browser's evaluation, which the page's policy does not
// cover), so it must be one expression. It runs only after page_script.js answered evalRefused (nothing ran then).
(() => {
  // eslint-disable-next-line no-undef -- JSON request token supplied by the Rust assembler.
  const req = __REQUEST__;
  if (location.href !== req.url) return JSON.stringify({error:"The page changed; read it before running JavaScript."});
  const result = {done:false};
  Object.defineProperty(window,req.key,{value:result,configurable:true});
  const finish = data => { if (!result.done) Object.assign(result,data,{done:true}); };
  const timer = setTimeout(() => finish({error:"The expression exceeded its completion deadline; it may still have pending effects"}),req.timeoutMs);
  Promise.resolve().then(() => (
// eslint-disable-next-line no-undef -- the Rust assembler replaces the next line with the agent's expression.
__EXPRESSION__
)).then(value => {
    try {
      const encoded=JSON.stringify(value === undefined ? null : value);
      if (encoded.length > 1800) finish({value:encoded.slice(0,1800),truncated:true,encoding:"jsonPrefix"});
      else finish({value:JSON.parse(encoded),truncated:false});
    } catch (error) { finish({error:`The return value is not JSON serializable: ${String(error).slice(0,500)}`}); }
  },error => finish({error:String(error).slice(0,1000)})).finally(() => clearTimeout(timer));
  return JSON.stringify({started:true});
})()
