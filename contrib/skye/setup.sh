#!/bin/sh

mkdb() {
	db=flock.db
	if [ ! -z "$1" ]; then
		db="$1"
	fi
	sqlite3 "$db" "`cat schema.sql`"
}

case "$1" in
	mkdb) {
		mkdb "$2"
	} ;;

	*) {
		cat > /dev/stdout << EOF
usage:
	$0 [cmd]

cmds:
	mkdb [p]	make a new database
			optional: specify path p
EOF
	} ;;

esac
