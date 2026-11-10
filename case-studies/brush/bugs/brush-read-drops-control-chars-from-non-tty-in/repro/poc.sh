read -r v <<< $'a\x01b'; printf '%s' "$v" | od -An -c
printf 'x\003y\n' | { read -r v; echo "status=$? v=$v"; }
printf 'x\004y\n' | { read -r v; echo "status=$? v=$v"; }
