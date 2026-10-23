b=(a b c d)
b[5]=q
echo "<${b[-2]}> <${b[-1]}>"
b[-2]=Z
declare -p b
unset 'b[-3]'
declare -p b
