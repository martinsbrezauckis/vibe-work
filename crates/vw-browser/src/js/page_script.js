// Explicitly authorized main-world expression, evaluated once with a bounded asynchronous completion record.
// Polling reads this record; it never re-evaluates the expression or exposes the trusted agent world.
(() => {
  // eslint-disable-next-line no-undef -- JSON request token supplied by the Rust assembler.
  const req = __REQUEST__;
  if (location.href !== req.url) return JSON.stringify({error:"The page changed; read it before running JavaScript."});
  const result = {done:false};
  Object.defineProperty(window,req.key,{value:result,configurable:true});
  const finish = data => { if (!result.done) Object.assign(result,data,{done:true}); };
  const timer = setTimeout(() => finish({error:"The expression exceeded its completion deadline; it may still have pending effects"}),req.timeoutMs);
  Promise.resolve().then(() => (0,eval)(req.expression)).then(value => {
    try {
      const encoded=JSON.stringify(value === undefined ? null : value);
      if (encoded.length > 1800) finish({value:encoded.slice(0,1800),truncated:true,encoding:"jsonPrefix"});
      else finish({value:JSON.parse(encoded),truncated:false});
    } catch (error) { finish({error:`The return value is not JSON serializable: ${String(error).slice(0,500)}`}); }
  },error => finish({error:String(error).slice(0,1000)})).finally(() => clearTimeout(timer));
  return JSON.stringify({started:true});
})()
