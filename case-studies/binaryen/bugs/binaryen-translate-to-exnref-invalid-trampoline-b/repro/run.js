const b = readbuffer(arguments[0]);
const i = new WebAssembly.Instance(new WebAssembly.Module(b));
try { i.exports.f(); print("returned"); } catch (e) { print("trap: " + e.message); }
