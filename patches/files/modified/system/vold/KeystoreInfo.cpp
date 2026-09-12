/*
		Copyright 2013 to 2022 TeamWin
		TWRP is free software: you can redistribute it and/or modify
		it under the terms of the GNU General Public License as published by
		the Free Software Foundation, either version 3 of the License, or
		(at your option) any later version.

		TWRP is distributed in the hope that it will be useful,
		but WITHOUT ANY WARRANTY; without even the implied warranty of
		MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
		GNU General Public License for more details.

		You should have received a copy of the GNU General Public License
		along with TWRP.  If not, see <http://www.gnu.org/licenses/>.
*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <string.h>
#include <fcntl.h>
#include <errno.h>
#include <ctype.h>
#include <string>
#include <sstream>
#include <algorithm>
#include <string>
#include <sqlite3.h>
#include "KeystoreInfo.hpp"

std::string KeystoreInfo::uint2hex(int64_t num) {
	uint64_t temp = num;
	std::string s = "";
	while (temp) {
		int a = temp % 16;
		if (a <= 9)
			s += (48 + a);
		else
			s += (87 + a);
		temp = temp / 16;
	}
	std::reverse(s.begin(), s.end());
	return s;
}

std::string KeystoreInfo::getHandle(const userid_t user_id) {
	int rc = 0;
	sqlite3 *db;
	sqlite3_stmt *stmt;
	char *err_msg = 0;

	rc = sqlite3_open("/data/system/locksettings.db", &db);
	if (rc) {
		fprintf(stderr, "Can't open database: %s\n", sqlite3_errmsg(db));
		return "";
	}
	std::string sql = "SELECT * FROM locksettings WHERE name = 'sp-handle' AND user = " + std::to_string(user_id);
	rc = sqlite3_prepare_v2(db, sql.c_str(), -1, &stmt, NULL);
	if (rc != SQLITE_OK) {
		sqlite3_close(db);
		return "";
	}
	int64_t value = 0;
	while ((rc = sqlite3_step(stmt)) == SQLITE_ROW) {
		value = sqlite3_column_int64(stmt, 3);
	}
	if (rc != SQLITE_DONE) {
		fprintf(stderr, "error: %s\n", sqlite3_errmsg(db));
		return "";
	}
	sqlite3_finalize(stmt);
	sqlite3_close(db);
	return uint2hex(value);
}

std::string KeystoreInfo::getAlias(std::string handle) {
	std::string alias(SYNTHETIC_PASSWORD_KEY_PREFIX);
	alias = alias + handle;
	return alias;
}
