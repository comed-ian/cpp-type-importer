#include <stdint.h>

struct Zyx_abc {
  int32_t a;
  int64_t b;
  bool* c;
};

struct ddd {
  void* (*f)(void*, int32_t);
};

template <typename T, typename UV> struct Abc {
  T* a;
  UV* *b;
};

Abc<int32_t, bool>;

Abc<Abc<int32_t, bool>, int32_t>;
