#include <stdint.h>

struct Zyx_abc {
  int32_t a;
  int64_t b;
  bool* c;
};

struct ddd {
  void* (*f)(void*, int32_t);
};

struct ccc {
  Zyx_abc* a;
};

template <typename T, typename UV> struct Abc {
  T* a;
  UV* *b;
};

Abc<int32_t, bool>;

Abc<Abc<int32_t, bool>, int32_t>;

Abc<uint32_t, uint32_t*>;

template <typename A> struct Def {
  Abc<A, A*> a;
};

Def<uint32_t>;

template <typename T> struct eee {
  void* (*fn)(T* clazz);
};

eee<Zyx_abc>;

enum fff : uint8_t {
  ZERO,
  ONE,
  TWO
};

enum ggg {
  GGGZERO=1,
  GGGONE,
  GGGTWO
};
