f() { echo "a
b"; }
declare -f f
eval "$(declare -f f)"
f
