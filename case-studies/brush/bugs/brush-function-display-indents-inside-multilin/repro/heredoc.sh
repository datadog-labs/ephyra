g() { cat <<EOF
hi
EOF
}
declare -f g
eval "$(declare -f g)"
g
