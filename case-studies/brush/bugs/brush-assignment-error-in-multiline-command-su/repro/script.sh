arr=(x)
v=$(
arr[-2]=y
echo "after subscript assignment"
)
echo "v=<$v> status=$?"

readonly ro=1
v=$(
ro=2
echo "after readonly assignment"
)
echo "v=<$v> status=$?"

v=$(
x=$((1/0))
echo "after arithmetic error"
)
echo "v=<$v> status=$?"
