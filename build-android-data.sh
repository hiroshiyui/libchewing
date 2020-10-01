#!/usr/bin/env sh
BASEDIR=$(dirname "$0")

mkdir -p $BASEDIR/build && cd $BASEDIR/build/
cmake ..
make
