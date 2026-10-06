#include "policy.h"
#include <cstdio>
int main() {
    eb_table table;
    eb_initialize(&table);
    const eb_identity host = {42, 1000};
    if (eb_classify(&table, host, 4, 1, 6) != EB_HOST) return 1;
    std::puts("POLICY_CXX_LINKAGE_PASS");
    return 0;
}
