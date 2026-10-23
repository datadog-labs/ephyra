b=(x)
f() { b[-5]=y; echo "in f"; }
f; echo "rc=$?"
echo after
