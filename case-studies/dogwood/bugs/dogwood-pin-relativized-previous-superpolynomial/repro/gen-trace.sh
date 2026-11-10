#!/usr/bin/env bash
# Prints N Write requests by one user, 1 s apart (default N=20).
N=${1:-20}
for i in $(seq 1 "$N"); do
  echo "@$i scope(principal: App::User::\"alice\", resource: App::Gateway::\"gw\") request_context(input: { user: \"alice\" }) App::Action::\"Write\"::request(input: { user: \"alice\" }, callerPrincipal: App::User::\"alice\", callerResource: App::Gateway::\"gw\", requestId: \"r$i\")"
done
