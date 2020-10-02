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

# build zip archive
cd $BASEDIR

FILES=("dictionary.dat" "index_tree.dat" "pinyin.tab" "swkb.dat" "symbols.dat")

for ((i=0; i < ${#FILES[@]}; i++))
do
  zip -j data.zip $BASEDIR/data/${FILES[$i]}
done
