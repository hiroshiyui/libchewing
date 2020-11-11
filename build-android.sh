#!/usr/bin/env bash
BASEDIR=$(dirname $(realpath "$0"))

# clean files
git clean -dfx

# init build
./autogen.sh && ./configure

# build init_database
cd $BASEDIR/src/tools
make

# build data files
cd $BASEDIR/data
make
