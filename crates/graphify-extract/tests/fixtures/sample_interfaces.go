package store

import (
	"errors"
	"fmt"
	"sync"
)

type Storable interface {
	Key() string
	Save() error
}

type Cacher interface {
	Storable
	Cache() bool
}

type User struct {
	ID   string
	Name string
}

func (u *User) Key() string {
	return u.ID
}

func (u *User) Save() error {
	if u.ID == "" {
		return errors.New("missing id")
	}
	fmt.Println("saving", u.Name)
	return nil
}

func (u *User) Cache() bool {
	return len(u.Name) > 0
}

type Store struct {
	mu    sync.RWMutex
	items map[string]Storable
}

func NewStore() *Store {
	return &Store{items: make(map[string]Storable)}
}

func (s *Store) Put(item Storable) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.items[item.Key()] = item
}

func (s *Store) Get(key string) (Storable, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	v, ok := s.items[key]
	return v, ok
}

func CheckCache(item Storable) bool {
	if c, ok := item.(Cacher); ok {
		return c.Cache()
	}
	return false
}

func main() {
	s := NewStore()
	u := &User{ID: "u1", Name: "alice"}
	if err := u.Save(); err != nil {
		fmt.Println(err)
	}
	s.Put(u)
	if got, ok := s.Get("u1"); ok {
		fmt.Println("got", got.Key())
	}
	fmt.Println("cacheable:", CheckCache(u))
}
