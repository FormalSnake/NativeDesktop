#!/bin/sh
# Answers any message with {"pong":true}: a 13-byte body behind its native-order length.
printf '\015\000\000\000{"pong":true}'
sleep 1
